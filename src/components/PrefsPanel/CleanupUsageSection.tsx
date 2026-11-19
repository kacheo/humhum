import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ipc } from "../../lib/ipc";
import type { CleanupUsage, ReportedTokenCounts } from "../../lib/types";

const monthFormat = new Intl.DateTimeFormat(undefined, { month: "long", year: "numeric", calendar: "gregory", timeZone: "UTC" });
const dayFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "long", calendar: "gregory", timeZone: "UTC" });
const suffixes = ["", "K", "M", "B", "T", " quadrillion", " quintillion"];

function compact(value: string) {
  const count = BigInt(value);
  if (count < 1000n) return count.toLocaleString();
  let unit = 1000n;
  let index = 1;
  while (index < suffixes.length - 1 && count >= unit * 1000n) {
    unit *= 1000n;
    index++;
  }
  let tenths = (count * 10n + unit / 2n) / unit;
  if (tenths === 10000n && index < suffixes.length - 1) {
    tenths = 10n;
    index++;
  }
  return (Number(tenths) / 10).toLocaleString(undefined, { maximumFractionDigits: 1 }) + suffixes[index];
}

function exact(count: string | null) {
  return count === null ? "Not reported" : BigInt(count).toLocaleString();
}

function monthStart(month: string) {
  return new Date(`${month}-01T00:00:00Z`);
}

function shiftMonth(month: string, offset: number) {
  const date = monthStart(month);
  date.setUTCMonth(date.getUTCMonth() + offset);
  return date.toISOString().slice(0, 7);
}

function noCounts(counts: ReportedTokenCounts) {
  return counts.inputTokens === null && counts.outputTokens === null;
}

function dayUsage(counts?: ReportedTokenCounts) {
  if (!counts) return "No cleanup";
  if (noCounts(counts)) return "No counts reported";
  return `Input ${exact(counts.inputTokens)}, Output ${exact(counts.outputTokens)}`;
}

function intensity(counts?: ReportedTokenCounts) {
  if (!counts) return "empty";
  if (noCounts(counts)) return "unreported";
  const combined = BigInt(counts.inputTokens ?? 0) + BigInt(counts.outputTokens ?? 0);
  if (combined < 1000n) return "low";
  if (combined < 10000n) return "medium";
  if (combined < 100000n) return "high";
  return "highest";
}

function Totals({ counts }: { counts: ReportedTokenCounts }) {
  return (
    <dl className="usage__totals">
      {([["Input", counts.inputTokens], ["Output", counts.outputTokens]] as const).map(([label, count]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd aria-label={exact(count)} title={exact(count)}>{count === null ? "—" : compact(count)}</dd>
        </div>
      ))}
    </dl>
  );
}

const weekdays = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const arrowOffsets: Record<string, number> = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 };

function UsageCalendar({ usage, labelId }: { usage: CleanupUsage; labelId: string }) {
  const { month, days } = usage;
  const start = monthStart(month).getUTCDay();
  const last = monthStart(month);
  last.setUTCMonth(last.getUTCMonth() + 1, 0);
  const dayCount = last.getUTCDate();
  const [selected, setSelected] = useState(
    () => Number(Object.keys(days).sort().pop()?.slice(-2) ?? 1),
  );
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  const key = (day: number) => `${month}-${String(day).padStart(2, "0")}`;
  const dateLabel = (day: number) => dayFormat.format(new Date(`${key(day)}T00:00:00Z`));
  const selectedCounts = days[key(selected)];
  return (
    <>
      <div role="grid" aria-labelledby={labelId} className="usage__calendar">
        <div role="row" className="usage__week">
          {weekdays.map((day) => <span key={day} role="columnheader" aria-label={day}>{day[0]}</span>)}
        </div>
        {Array.from({ length: Math.ceil((start + dayCount) / 7) }, (_, week) => (
          <div role="row" className="usage__week" key={week}>
            {Array.from({ length: 7 }, (_, column) => {
              const day = week * 7 + column - start + 1;
              if (day < 1 || day > dayCount) return <span role="gridcell" key={column} />;
              const counts = days[key(day)];
              return (
                <div role="gridcell" aria-selected={selected === day} key={column}>
                  <button
                    type="button"
                    className="usage__day"
                    data-intensity={intensity(counts)}
                    ref={(element) => { buttons.current[day] = element; }}
                    tabIndex={selected === day ? 0 : -1}
                    aria-label={`${dateLabel(day)}. ${dayUsage(counts)}`}
                    onClick={() => setSelected(day)}
                    onKeyDown={(event) => {
                      const offset = arrowOffsets[event.key];
                      if (!offset) return;
                      event.preventDefault();
                      const next = day + offset;
                      if (next < 1 || next > dayCount) return;
                      setSelected(next);
                      buttons.current[next]?.focus();
                    }}
                  >{day}</button>
                </div>
              );
            })}
          </div>
        ))}
      </div>
      <div className="usage__legend">
        <span><i data-intensity="empty" /> No cleanup</span>
        <span><i data-intensity="unreported" /> No counts reported</span>
        <span>
          Fewer tokens
          <i data-intensity="low" /><i data-intensity="medium" /><i data-intensity="high" /><i data-intensity="highest" />
          More
        </span>
      </div>
      <p className="usage__detail" role="status">
        <strong>{dateLabel(selected)}</strong> · {dayUsage(selectedCounts)}
      </p>
    </>
  );
}

export function CleanupUsageSection() {
  const labelId = useId();
  const monthHeading = useRef<HTMLHeadingElement>(null);
  const focusMonth = useRef(false);
  const [history, setHistory] = useState<string>();
  const [usage, setUsage] = useState<CleanupUsage>();
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let active = true;
    let request = 0;
    let unsubscribe: (() => void) | undefined;
    let midnight: ReturnType<typeof setTimeout>;
    const refresh = () => {
      if (!active) return;
      const version = ++request;
      void ipc.getCleanupUsage(history).then(
        (summary) => {
          if (!active || request !== version) return;
          setUsage(summary);
          setFailed(false);
        },
        () => { if (active && request === version) setFailed(true); },
      );
    };
    const scheduleMidnight = () => {
      clearTimeout(midnight);
      const now = new Date();
      const next = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
      midnight = setTimeout(() => {
        refresh();
        scheduleMidnight();
      }, next.getTime() - now.getTime());
    };
    const onFocus = () => {
      refresh();
      scheduleMidnight();
    };
    window.addEventListener("focus", onFocus);
    scheduleMidnight();
    void listen("cleanup-usage-updated", refresh).then((stop) => {
      if (!active) {
        stop?.();
        return;
      }
      unsubscribe = stop;
      refresh();
    }, refresh);
    return () => {
      active = false;
      unsubscribe?.();
      window.removeEventListener("focus", onFocus);
      clearTimeout(midnight);
    };
  }, [history]);

  useLayoutEffect(() => {
    if (focusMonth.current) {
      monthHeading.current?.focus({ preventScroll: true });
      focusMonth.current = false;
    }
  }, [history]);

  const month = history ?? usage?.currentMonth;
  const visible = usage?.month === month ? usage : undefined;
  const previous = month && shiftMonth(month, -1);
  const next = month && shiftMonth(month, 1);
  const current = usage?.currentMonth;
  const canGoBack = previous && usage?.firstMonth && previous >= usage.firstMonth;
  const canGoForward = next && current && next <= current;
  const goTo = (target: string, focusHeading: boolean) => {
    focusMonth.current = focusHeading;
    setFailed(false);
    setHistory(target === current ? undefined : target);
  };

  return (
    <section className="prefs__section usage">
      <h2 className="prefs__heading">Token usage</h2>
      {month && (
        <div className="usage__navigation">
          <h3 id={labelId} ref={monthHeading} tabIndex={-1} aria-live="polite" aria-atomic="true">{monthFormat.format(monthStart(month))}</h3>
          {history && <button type="button" onClick={() => goTo(current!, true)}>This month</button>}
          <div className="usage__arrows">
            <button type="button" aria-label="Previous month" disabled={!canGoBack} onClick={() => goTo(previous!, previous === usage?.firstMonth)}>‹</button>
            <button type="button" aria-label="Next month" disabled={!canGoForward} onClick={() => goTo(next!, next === current)}>›</button>
          </div>
        </div>
      )}
      {failed ? <p className="prefs__hint" role="status">Usage unavailable</p> : !visible ? (
        <p className="prefs__hint" role="status">Loading usage…</p>
      ) : !visible.firstMonth || visible.month < visible.firstMonth ? (
        <p className="prefs__hint" role="status">Daily usage starts with your next cleanup.</p>
      ) : (
        <>
          <Totals counts={visible.totals} />
          {Object.keys(visible.days).length === 0 ? (
            <p className="prefs__hint">No cleanup this month</p>
          ) : noCounts(visible.totals) && (
            <p className="prefs__hint">No counts reported</p>
          )}
          <UsageCalendar key={month} usage={visible} labelId={labelId} />
        </>
      )}
      {usage && !failed && (
        <details className="usage__all-time">
          <summary>All-time usage</summary>
          <Totals counts={usage.lifetime} />
        </details>
      )}
    </section>
  );
}
