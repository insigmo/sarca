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

  const media = bindings.find((b) => b.mode === "auto_upload");
  $("mediaStatus").textContent = media
    ? `On → ${media.local_path} (remote ${media.remote_root || "Media"})`
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
      const path = await invoke("pick_local_folder");
      if (path) $("localPath").value = path;
    } catch (e) {
      setMsg(String(e));
    }
  };

  $("enableMedia").onclick = async () => {
    try {
      const path = await invoke("pick_local_folder");
      if (!path) return;
      const storageId = $("storageSelect").value;
      if (!storageId) throw new Error("Select a storage first");
      const remote = await invoke("ensure_remote_folder", {
        storageId,
        parent: "",
        name: "Media",
      });
      const bindings = await invoke("list_bindings");
      for (const b of bindings.filter((x) => x.mode === "auto_upload")) {
        await invoke("remove_binding", { id: b.id });
      }
      await invoke("add_binding", {
        storageId,
        remoteRoot: String(remote).replace(/\/$/, "") || "Media",
        localPath: path,
        mode: "auto_upload",
      });
      await refreshBindings();
    } catch (e) {
      setMsg(String(e));
    }
  };

  $("disableMedia").onclick = async () => {
    try {
      const bindings = await invoke("list_bindings");
      for (const b of bindings.filter((x) => x.mode === "auto_upload")) {
        await invoke("remove_binding", { id: b.id });
      }
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
      const localPath = $("localPath").value.trim();
      const remoteRoot = $("remoteRoot").value.trim().replace(/\/$/, "");
      if (!storageId) throw new Error("Select a storage");
      if (!localPath) throw new Error("Pick a local folder");
      if (!remoteRoot) throw new Error("Set a remote folder path or create one");
      await invoke("add_binding", {
        storageId,
        remoteRoot,
        localPath,
        mode: "sync",
      });
      $("localPath").value = "";
      await refreshBindings();
    } catch (e) {
      setMsg(String(e));
    }
  };
});
