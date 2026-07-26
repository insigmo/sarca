import { invoke } from "@tauri-apps/api/core";

const $ = (id) => document.getElementById(id);

function setError(message) {
  $("error").textContent = message || "";
}

function setSyncEnabled(connected) {
  const btn = $("openSync");
  if (!btn) return;
  btn.disabled = !connected;
  btn.title = connected
    ? "Media auto-upload and folder sync"
    : "Connect first to manage Sync";
}

window.addEventListener("DOMContentLoaded", async () => {
  try {
    $("platform").textContent = await invoke("platform_label");
  } catch {
    $("platform").textContent = "";
  }

  try {
    const session = await invoke("get_session");
    if (session?.base_url) {
      $("serverUrl").value = session.base_url;
    }
    if (session?.email) {
      $("email").value = session.email;
    }
    setSyncEnabled(Boolean(session?.connected));
    // Saved session auto-opens the server app from Rust on launch.
  } catch (e) {
    setError(String(e));
    setSyncEnabled(false);
  }

  $("openSync")?.addEventListener("click", async () => {
    setError("");
    try {
      await invoke("open_sync_settings");
    } catch (e) {
      setError(String(e).replace(/^Error:\s*/i, ""));
    }
  });

  $("connectForm").addEventListener("submit", async (event) => {
    event.preventDefault();
    setError("");
    const button = $("submit");
    button.disabled = true;
    button.textContent = "Connecting…";
    try {
      await invoke("connect", {
        serverUrl: $("serverUrl").value.trim(),
        email: $("email").value.trim(),
        password: $("password").value,
      });
      setSyncEnabled(true);
      // Rust navigates the webview to the server UI.
    } catch (e) {
      setError(String(e).replace(/^Error:\s*/i, ""));
      button.disabled = false;
      button.textContent = "Connect";
    }
  });
});
