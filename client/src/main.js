import { invoke } from "@tauri-apps/api/core";

const $ = (id) => document.getElementById(id);

function setError(message) {
  $("error").textContent = message || "";
}

function renderUrlHistory(urls) {
  const wrap = $("urlHistory");
  const list = $("urlHistoryList");
  list.innerHTML = "";
  if (!urls?.length) {
    wrap.hidden = true;
    return;
  }
  for (const url of urls) {
    const item = document.createElement("button");
    item.type = "button";
    item.className = "url-history__item";
    item.textContent = url;
    item.addEventListener("click", () => {
      const input = $("serverUrl");
      input.value = url;
      input.focus();
    });
    list.appendChild(item);
  }
  wrap.hidden = false;
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

  try {
    renderUrlHistory(await invoke("get_url_history"));
  } catch {
    renderUrlHistory([]);
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
