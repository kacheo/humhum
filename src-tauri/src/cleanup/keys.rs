use std::sync::Mutex;

use super::ApiProvider;
use crate::error::AppError;

/// Per-provider ordering survives editor remounts. The mutex is never held over
/// a network await; it only guards issuing tokens and committing credentials.
#[derive(Default)]
pub struct KeyActions(Mutex<[u64; 2]>);

pub(crate) struct KeyAction {
    index: usize,
    generation: u64,
}

impl KeyActions {
    pub(crate) fn begin(&self, provider: ApiProvider) -> KeyAction {
        let index = match provider {
            ApiProvider::OpenAi => 0,
            ApiProvider::Anthropic => 1,
        };
        let mut generations = self.0.lock().unwrap();
        generations[index] += 1;
        KeyAction {
            index,
            generation: generations[index],
        }
    }

    pub(crate) fn complete(
        &self,
        action: KeyAction,
        commit: impl FnOnce() -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let generations = self.0.lock().unwrap();
        if generations[action.index] != action.generation {
            return Err(AppError::Cleanup("key_action_superseded".into()));
        }
        // Keep the guard through commit, so a newer action cannot start between
        // the generation check and the actual Keychain mutation.
        commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delayed_verification_cannot_resurrect_a_removed_key() {
        let actions = KeyActions::default();
        let credential = std::sync::Mutex::new(Some("saved-key"));
        let old = actions.begin(ApiProvider::OpenAi);
        let (released, wait) = tokio::sync::oneshot::channel();
        let delayed = async {
            wait.await.unwrap();
            actions.complete(old, || {
                *credential.lock().unwrap() = Some("late-key");
                Ok(())
            })
        };
        let remove = async {
            let removal = actions.begin(ApiProvider::OpenAi);
            actions
                .complete(removal, || {
                    *credential.lock().unwrap() = None;
                    Ok(())
                })
                .unwrap();
            released.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(delayed, remove);
        assert_eq!(result.unwrap_err().code(), "key_action_superseded");
        assert_eq!(*credential.lock().unwrap(), None);
    }

    #[test]
    fn older_verification_cannot_overwrite_a_newer_replacement() {
        let actions = KeyActions::default();
        let mut credential = "original";
        let old = actions.begin(ApiProvider::OpenAi);
        let new = actions.begin(ApiProvider::OpenAi);
        actions
            .complete(new, || {
                credential = "new-key";
                Ok(())
            })
            .unwrap();
        let result = actions.complete(old, || {
            credential = "old-key";
            Ok(())
        });
        assert_eq!(result.unwrap_err().code(), "key_action_superseded");
        assert_eq!(credential, "new-key");
    }

    #[test]
    fn providers_have_independent_actions() {
        let actions = KeyActions::default();
        let openai = actions.begin(ApiProvider::OpenAi);
        let anthropic = actions.begin(ApiProvider::Anthropic);
        assert!(actions.complete(openai, || Ok(())).is_ok());
        assert!(actions.complete(anthropic, || Ok(())).is_ok());
    }
}
