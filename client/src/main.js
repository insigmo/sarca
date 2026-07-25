import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

const $ = (id) => document.getElementById(id);

async function refresh() {
  const bindings = await invoke("list_bindings");
  const el = $("bindings");
  el.innerHTML = "";
  if (!bindings.length) {
    el.innerHTML = `<p class="status">No bindings yet.</p>`;
  } else {
    for (const b of bindings) {
      const row = document.createElement("div");
      row.className = "binding";
      row.innerHTML = `
        <div>
          <div><strong>${b.mode}</strong> → ${b.local_path}</div>
          <div class="status">${b.storage_id}${b.remote_root ? " / " + b.remote_root : ""}</div>
        </div>
        <button class="secondary" data-id="${b.id}" style="width:auto">Remove</button>
      `;
      row.querySelector("button").onclick = async () => {
        await invoke("remove_binding", { id: b.id });
        await refresh();
      };
      el.appendChild(row);
    }
  }
  const statuses = await invoke("sync_statuses");
  $("status").textContent = JSON.stringify(statuses, null, 2);
}

async function loadServer() {
  const cfg = await invoke("get_server_config");
  $("baseUrl").value = cfg.base_url || "";
  $("token").value = cfg.access_token || "";
}

window.addEventListener("DOMContentLoaded", async () => {
  try {
    $("platform").textContent = await invoke("platform_label");
    await loadServer();
    await refresh();
  } catch (e) {
    $("status").textContent = String(e);
  }

  $("saveServer").onclick = async () => {
    await invoke("set_server_config", {
      baseUrl: $("baseUrl").value.trim(),
      accessToken: $("token").value.trim(),
    });
    $("status").textContent = "Server saved.";
  };

  $("pickFolder").onclick = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (selected) $("localPath").value = selected;
  };

  $("pickCamera").onclick = async () => {
    const selected = await open({ directory: true, multiple: false, title: "Photos / camera folder" });
    if (selected) {
      $("localPath").value = selected;
      $("mode").value = "auto_upload";
    }
  };

  $("addBinding").onclick = async () => {
    await invoke("add_binding", {
      storageId: $("storageId").value.trim(),
      remoteRoot: $("remoteRoot").value.trim(),
      localPath: $("localPath").value.trim(),
      mode: $("mode").value,
    });
    $("storageId").value = "";
    await refresh();
  };

  $("tickNow").onclick = async () => {
    await invoke("sync_now");
    await refresh();
  };

  setInterval(() => {
    refresh().catch(() => {});
  }, 5000);
});
