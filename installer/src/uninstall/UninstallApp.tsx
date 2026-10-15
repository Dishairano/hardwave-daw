import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { onUninstallProgress, quit, startUninstall, uninstallSteps } from "../lib/api";
import "./uninstall.css";

/**
 * The removal window: the approved uninstaller design
 * (suite.hardwavestudios.com/uninstaller-mockup/?app=daw). Remove? with
 * what stays and what goes and one option, off by default; Removing;
 * Removed; Cannot remove yet. Nothing is removed before the button.
 */

type Screen = "confirm" | "removing" | "done" | "error";

const KEEP = [
  "Your projects, samples and exports",
  "Your FL Studio projects",
  "Your Hardwave plug-ins in other DAWs",
];
const GOES = ["The DAW and its built-in plug-ins", "Its shortcuts", "Its entry in Windows Apps"];
const FALLBACK_STEPS = [
  "Close Hardwave DAW",
  "Remove the app",
  "Remove the shortcuts",
  "Remove it from Windows Apps",
];

/** How far along a step is, for the bar: the last step ends at 100. */
function percentAt(step: number, count: number): number {
  return Math.round(((step + 0.5) / count) * 90);
}

/** What the last screen says for a stop the backend reported. */
function stopText(code: string): { title: string; sub: string } {
  if (code === "daw-running") {
    return {
      title: "Hardwave DAW is still open.",
      sub: "Save your project and close the DAW, then press Try again.",
    };
  }
  if (code === "not-installed") {
    return {
      title: "Hardwave DAW is not installed here.",
      sub: "There is nothing to remove. If it still shows in Windows Apps, reinstall it from hardwavestudios.com/download and remove it again.",
    };
  }
  return { title: "Hardwave DAW was not removed.", sub: code };
}

// The spectrum under the done screen, as in the setup window.
const BARS = Array.from({ length: 32 }, (_, i) => {
  const a = 0.12 + ((i * 37) % 38) / 100;
  const b = 0.52 + ((i * 53) % 45) / 100;
  return { a, b };
});

export function UninstallApp() {
  const [screen, setScreen] = useState<Screen>("confirm");
  const [removeSettings, setRemoveSettings] = useState(false);
  const [steps, setSteps] = useState<string[]>(FALLBACK_STEPS);
  const [step, setStep] = useState(0);
  const [log, setLog] = useState("");
  const [stop, setStop] = useState("daw-running");

  useEffect(() => {
    uninstallSteps().then(setSteps).catch(() => {});
    let unlisten: (() => void) | undefined;
    onUninstallProgress((p) => {
      setStep(p.step);
      setLog(p.message);
    }).then((u) => {
      unlisten = u;
    });
    return () => unlisten?.();
  }, []);

  const remove = async () => {
    setStep(0);
    setLog("");
    setScreen("removing");
    try {
      await startUninstall(removeSettings);
      setScreen("done");
    } catch (e) {
      setStop(String(e));
      setScreen("error");
    }
  };

  const win = getCurrentWindow();
  const busy = screen === "removing";
  const percent = percentAt(step, steps.length);
  const failed = stopText(stop);

  return (
    <div className="un">
      <div className="itb" data-tauri-drag-region>
        <span data-tauri-drag-region>HARDWAVE DAW · REMOVE</span>
        <span className="c">
          <button type="button" title="Minimize" onClick={() => win.minimize()}>
            –
          </button>
          {/* Closing mid-removal would leave the window gone and the
              removal running; it closes once the removal is done. */}
          <button type="button" className="x" title="Close" disabled={busy} onClick={() => quit()}>
            ×
          </button>
        </span>
      </div>

      {screen === "confirm" && (
        <div className="screen">
          <div className="ib">
            <div className="cap">Remove</div>
            <h1 className="t">Remove Hardwave DAW?</h1>
            <p className="sub">
              Your projects stay where you saved them. This only removes the DAW from this computer.
            </p>
            <div className="lists">
              <div className="lst keep">
                <div className="k">Stays</div>
                <ul>
                  {KEEP.map((x) => (
                    <li key={x}>{x}</li>
                  ))}
                </ul>
              </div>
              <div className="lst go">
                <div className="k">Goes</div>
                <ul>
                  {GOES.map((x) => (
                    <li key={x}>{x}</li>
                  ))}
                </ul>
              </div>
            </div>
            <button
              type="button"
              className="opt"
              role="checkbox"
              aria-checked={removeSettings}
              onClick={() => setRemoveSettings((v) => !v)}
            >
              <i className={removeSettings ? "on" : ""} />
              <span>
                Also remove my DAW settings and recent files list
                <small>Off by default.</small>
              </span>
            </button>
            <div className="ft">
              <button type="button" className="ibtn ghost" onClick={() => quit()}>
                Cancel
              </button>
              <button type="button" className="ibtn danger" onClick={remove}>
                Remove DAW
              </button>
            </div>
          </div>
        </div>
      )}

      {screen === "removing" && (
        <div className="screen">
          <div className="ib">
            <div className="cap">Removing · {percent}%</div>
            <h1 className="t">Taking it off this computer.</h1>
            <div className="bar">
              <i style={{ width: `${percent}%` }} />
            </div>
            <div className="ph">
              {steps.map((s, i) => (
                <div key={s} className={i < step ? "d" : i === step ? "n" : "w"}>
                  {s}
                </div>
              ))}
            </div>
            <div className="logrow">{log || "Starting…"}</div>
          </div>
        </div>
      )}

      {screen === "done" && (
        <div className="screen">
          <div className="ib center">
            <svg className="bigmark" viewBox="0 0 200 200" aria-hidden="true">
              <defs>
                <linearGradient id="wv" x1="40" y1="100" x2="160" y2="100" gradientUnits="userSpaceOnUse">
                  <stop offset="0" stopColor="#dc2626" stopOpacity=".35" />
                  <stop offset=".15" stopColor="#dc2626" />
                  <stop offset=".5" stopColor="#ef4444" />
                  <stop offset=".85" stopColor="#dc2626" />
                  <stop offset="1" stopColor="#dc2626" stopOpacity=".35" />
                </linearGradient>
              </defs>
              <rect x="20" y="20" width="160" height="160" rx="34" fill="#151517" stroke="rgba(255,255,255,.08)" />
              <path
                d="M40 100 H52 L58 93 L66 58 L74 138 L82 68 L88 130 L94 86 L100 114 L106 94 L112 106 L118 100 H160"
                stroke="url(#wv)"
                strokeWidth="7"
                strokeLinecap="round"
                strokeLinejoin="round"
                fill="none"
              />
            </svg>
            <div className="wm">
              <b>HARDWAVE</b>
              <i>DAW</i>
            </div>
            <div className="cap ok" style={{ marginTop: 10 }}>
              Removed
            </div>
            <h1 className="t big">Hardwave DAW is removed.</h1>
            <p className="sub">
              Your projects are where you saved them. Want it back? hardwavestudios.com/download
            </p>
            <div className="ft center">
              <button type="button" className="ibtn" onClick={() => quit()}>
                Close
              </button>
            </div>
          </div>
          <div className="ibars">
            {BARS.map((b, i) => (
              <i key={i} style={{ ["--a" as string]: b.a, ["--b" as string]: b.b }} />
            ))}
          </div>
        </div>
      )}

      {screen === "error" && (
        <div className="screen">
          <div className="ib">
            <div className="cap bad">Not removed yet</div>
            <h1 className="t">{failed.title}</h1>
            <p className="sub">{failed.sub}</p>
            <div className="ft">
              <button type="button" className="ibtn ghost" onClick={() => quit()}>
                Close
              </button>
              <button type="button" className="ibtn" onClick={remove}>
                Try again
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
