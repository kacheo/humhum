# Third-party components

Wipzi's source is AGPLv3. Its dependencies retain their own licenses. The Rust and npm lockfiles record exact versions; inspect the applicable dependency licenses before redistributing a build.

Whisper `ggml-base.en.bin` and `ggml-small.en.bin` are downloaded on demand from [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp), whose model card identifies the models as MIT licensed. They are not included in this repository or installer. Wipzi also connects to optional Ollama, OpenAI, and Anthropic services chosen by the user; those services have their own terms and privacy policies.

The Rust and npm lockfiles are the dependency inventory for a source checkout. Before the first public binary release, verify the exact license and notice obligations for the shipped dependency set. The lockfiles also include macOS test tooling and packages for other targets that are not bundled into the macOS app.
