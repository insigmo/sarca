import { invoke } from "@tauri-apps/api/core";

const $ = (id) => document.getElementById(id);

function setMsg(text) {
  $("status").textContent = text || "";
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function isMobilePlatform() {
  const label = ($("platformHint")?.dataset?.platform || "").toLowerCase();
  return label === "android" || label === "ios";
}

function cameraBinding(bindings) {
  return (Array.isArray(bindings) ? bindings : []).find(
    (b) => b?.mode === "auto_upload",
  );
}

async function enableBackgroundSync() {
  // If loading existing prefs fails, skip rather than call set_client_prefs
  // with a bare `{ background_sync: true }` — that would silently wipe any
  // other saved prefs (e.g. app_lock_enabled, cache_limit_bytes) on disk.
  let prefs;
  try {
    prefs = await invoke("get_client_prefs");
  } catch {
    return;
  }
  if (!prefs || typeof prefs !== "object") return;
  await invoke("set_client_prefs", {
    prefs: { ...prefs, background_sync: true },
  });
}

/** Prefer system folder picker; typed path only when native signals FOLDER_PICKER_USE_PROMPT. */
async function chooseLocalFolder(existing) {
  try {
    const path = await invoke("pick_local_folder");
    if (path) return path;
    return null;
  } catch (e) {
    const msg = String(e?.message || e || "");
    if (!/FOLDER_PICKER_USE_PROMPT/i.test(msg)) {
      if (/cancel/i.test(msg)) return null;
      setMsg(msg);
      throw e instanceof Error ? e : new Error(msg);
    }
  }
  const hint = isMobilePlatform()
    ? "Folder picker could not resolve a filesystem path. Enter a local folder path, e.g. /storage/emulated/0/DCIM or /storage/emulated/0/Pictures"
    : "Enter a local folder path";
  const typed = window.prompt(hint, existing || "");
  return typed && typed.trim() ? typed.trim() : null;
}

async function refreshStorages() {
  const storages = await invoke("list_storages");
  const sel = $("storageSelect");
  sel.innerHTML = "";
  for (const s of storages) {
    const opt = document.createElement("option");
    opt.value = s.id;
    opt.textContent = s.name;
    sel.appendChild(opt);
  }
  if (!storages.length) {
    setMsg("No storages available. Connect and open the app first.");
  }
}

async function refreshBindings() {
  const bindings = await invoke("list_bindings");
  const host = $("bindings");
  host.innerHTML = "";

  const media = cameraBinding(bindings);
  $("mediaStatus").textContent = media?.enabled
    ? `On → ${media.local_path} (remote ${media.remote_root || "Camera"})`
    : "Off";

  if (!bindings.length) {
    host.innerHTML = `<p class="muted">No bindings yet.</p>`;
  } else {
    for (const b of bindings) {
      const row = document.createElement("div");
      row.className = "binding";
      row.innerHTML = `
        <div>
          <strong>${escapeHtml(b.mode)}</strong>
          <div class="muted">${escapeHtml(b.local_path)}</div>
          <div class="muted">${escapeHtml(b.storage_id)} / ${escapeHtml(b.remote_root || "(root)")}</div>
        </div>
        <button type="button" data-id="${escapeHtml(b.id)}" class="btn secondary danger">Remove</button>
      `;
      row.querySelector("button").onclick = async () => {
        await invoke("remove_binding", { id: b.id });
        await refreshBindings();
      };
      host.appendChild(row);
    }
  }

  try {
    const statuses = await invoke("sync_statuses");
    setMsg(JSON.stringify(statuses, null, 2));
  } catch (e) {
    setMsg(String(e));
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  try {
    const label = await invoke("platform_label");
    const hint = $("platformHint");
    if (hint) {
      hint.dataset.platform = label || "";
      if (label === "Android" || label === "iOS") {
        hint.hidden = false;
        hint.textContent =
          "On this device, type a folder path (Browse may be unavailable). Common Android paths: /storage/emulated/0/DCIM, /storage/emulated/0/Pictures, /storage/emulated/0/Download";
      }
    }
  } catch {
    // ignore
  }

  try {
    await refreshStorages();
    await refreshBindings();
  } catch (e) {
    setMsg(String(e));
  }

  $("backToApp").onclick = async () => {
    try {
      await invoke("open_app");
    } catch (e) {
      setMsg(String(e));
    }
  };

  $("syncNow").onclick = async () => {
    try {
      await invoke("sync_now");
      await refreshBindings();
    } catch (e) {
      setMsg(String(e));
    }
  };

  $("pickLocal").onclick = async () => {
    try {
      const path = await chooseLocalFolder($("localPath").value.trim());
      if (path) $("localPath").value = path;
    } catch (e) {
      setMsg(String(e));
    }
  };

  $("enableMedia").onclick = async () => {
    try {
      const bindings = await invoke("list_bindings");
      const existing = cameraBinding(bindings);

      if (existing?.enabled === true) {
        await refreshBindings();
        return;
      }

      if (existing) {
        await invoke("set_binding_enabled", { id: existing.id, enabled: true });
        await enableBackgroundSync();
        await refreshBindings();
        invoke("sync_now")
          .then(() => refreshBindings())
          .catch((syncErr) => setMsg(String(syncErr)));
        return;
      }

      const path = await chooseLocalFolder($("localPath").value.trim());
      if (!path) return;
      $("localPath").value = path;
      const storageId = $("storageSelect").value;
      if (!storageId) throw new Error("Select a storage first");
      const remote = await invoke("ensure_remote_folder", {
        storageId,
        parent: "",
        name: "Camera",
      });
      await invoke("add_binding", {
        storageId,
        remoteRoot: String(remote).replace(/\/$/, "") || "Camera",
        localPath: path,
        mode: "auto_upload",
      });
      await enableBackgroundSync();
      await refreshBindings();
      // Fire-and-forget: awaiting sync_now kept the UI stuck for the whole upload.
      invoke("sync_now")
        .then(() => refreshBindings())
        .catch((syncErr) => setMsg(String(syncErr)));
    } catch (e) {
      setMsg(String(e));
    }
  };

  $("disableMedia").onclick = async () => {
    try {
      const bindings = await invoke("list_bindings");
      const existing = cameraBinding(bindings);
      if (!existing) {
        await refreshBindings();
        return;
      }
      await invoke("set_binding_enabled", { id: existing.id, enabled: false });
      await refreshBindings();
    } catch (e) {
      setMsg(String(e));
    }
  };

  $("createRemote").onclick = async () => {
    try {
      const storageId = $("storageSelect").value;
      const name = $("newFolderName").value.trim();
      const parent = $("remoteRoot").value.trim().replace(/\/$/, "");
      if (!storageId || !name) throw new Error("Storage and folder name required");
      const remote = await invoke("ensure_remote_folder", {
        storageId,
        parent,
        name,
      });
      $("remoteRoot").value = String(remote).replace(/\/$/, "");
      $("newFolderName").value = "";
      setMsg(`Created ${remote}`);
    } catch (e) {
      setMsg(String(e));
    }
  };

  $("addSync").onclick = async () => {
    try {
      const storageId = $("storageSelect").value;
      let localPath = $("localPath").value.trim();
      if (!localPath) {
        localPath = (await chooseLocalFolder("")) || "";
        if (localPath) $("localPath").value = localPath;
      }
      const remoteRoot = $("remoteRoot").value.trim().replace(/\/$/, "");
      if (!storageId) throw new Error("Select a storage");
      if (!localPath) throw new Error("Set a local folder path");
      if (!remoteRoot) throw new Error("Set a remote folder path or create one");
      await invoke("add_binding", {
        storageId,
        remoteRoot,
        localPath,
        mode: "folder_upload",
      });
      $("localPath").value = "";
      await refreshBindings();
      invoke("sync_now")
        .then(() => refreshBindings())
        .catch((syncErr) => setMsg(String(syncErr)));
    } catch (e) {
      setMsg(String(e));
    }
  };
});
