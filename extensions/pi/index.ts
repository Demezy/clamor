import { spawn } from "node:child_process";

/**
 * Pi extension that reports session ID to clamor.
 *
 * On session_start, pipes the session ID to `clamor hook` so clamor
 * can store it as resume_token for later reload/resume.
 *
 * Requires CLAMOR_AGENT_ID in the environment (set automatically by clamor
 * when spawning agents). Silently no-ops if clamor isn't available.
 *
 * Install: add this extension path to ~/.pi/agent/settings.json:
 *   { "extensions": ["/path/to/clamor/extensions/pi"] }
 * Or symlink into ~/.pi/agent/extensions/clamor-session/
 */
export default function (pi: any) {
  pi.on("session_start", async (_event: any, ctx: any) => {
    if (!process.env.CLAMOR_AGENT_ID) return;

    const sessionId = ctx.sessionManager.getSessionId();
    if (!sessionId) return;

    const payload = JSON.stringify({
      hook_event_name: "SessionStart",
      session_id: sessionId,
    });

    try {
      const child = spawn("clamor", ["hook"], {
        stdio: ["pipe", "ignore", "ignore"],
        env: process.env,
      });
      child.stdin.write(payload);
      child.stdin.end();
      // Don't wait — fire and forget so we don't block pi
    } catch {
      // clamor not installed or not in PATH — silently ignore
    }
  });
}
