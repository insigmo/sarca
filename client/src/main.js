import { invoke } from "@tauri-apps/api/core";

const $ = (id) => document.getElementById(id);

function setError(message) {
  $("error").textContent = message || "";
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
    // Saved session with tokens auto-opens the server app from Rust on launch.
  } catch (e) {
    setError(String(e));
  }

  $("connectForm").addEventListener("submit", async (event) => {
    event.preventDefault();
    setError("");
    const button = $("submit");
    button.disabled = true;
    button.textContent = "Connecting…";
    try {
      await invoke("connect", {
        serverUrl: $("serverUrl").value.trim(),
      });
      // Rust navigates the webview to the server UI; sign-in is on the website.
    } catch (e) {
      setError(String(e).replace(/^Error:\s*/i, ""));
      button.disabled = false;
      button.textContent = "Connect";
    }
  });
});
