const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);

const addr = $("addr");
const panel = $("panel");
const vpnBtn = $("vpn");
const coreState = $("coreState");
const modeState = $("modeState");
const groupsEl = $("groups");

let panelOpen = false;
let currentMode = "direct";

const TOOLBAR_H = 46;
const PANEL_H = 460;

async function go() {
  const url = addr.value.trim();
  if (!url) return;
  try {
    await invoke("navigate", { url });
  } catch (e) {
    console.error(e);
  }
}

$("go").addEventListener("click", go);
addr.addEventListener("keydown", (e) => {
  if (e.key === "Enter") go();
});
$("back").addEventListener("click", () => invoke("go_back").catch(console.error));
$("forward").addEventListener("click", () => invoke("go_forward").catch(console.error));
$("reload").addEventListener("click", () => invoke("reload").catch(console.error));

window.__onNav = (url) => {
  if (url && url !== "about:blank") addr.value = url;
};

async function setPanel(open) {
  panelOpen = open;
  panel.classList.toggle("open", open);
  await invoke("set_ui_height", { height: open ? PANEL_H : TOOLBAR_H });
  if (open) {
    refreshCoreState();
    loadProxies();
  }
}

$("settings").addEventListener("click", () => setPanel(!panelOpen));

vpnBtn.addEventListener("click", async () => {
  const next = currentMode === "direct" ? "rule" : "direct";
  await applyMode(next);
});

async function applyMode(mode) {
  try {
    await invoke("set_mode", { mode });
    currentMode = mode;
    vpnBtn.classList.toggle("active", mode !== "direct");
    modeState.textContent = `当前: ${mode}`;
    document.querySelectorAll(".mode").forEach((b) => {
      b.classList.toggle("active", b.dataset.mode === mode);
    });
  } catch (e) {
    modeState.textContent = "切换失败: " + e;
  }
}

document.querySelectorAll(".mode").forEach((b) => {
  b.addEventListener("click", () => applyMode(b.dataset.mode));
});

$("import").addEventListener("click", async () => {
  const text = $("configText").value.trim();
  if (!text) {
    alert("请粘贴 Clash 配置内容");
    return;
  }
  try {
    await invoke("import_config", { content: text });
    alert("导入成功");
    loadProxies();
  } catch (e) {
    alert("导入失败: " + e);
  }
});

$("sub").addEventListener("click", async () => {
  const url = $("subUrl").value.trim();
  if (!url) {
    alert("请填写订阅链接");
    return;
  }
  try {
    $("sub").textContent = "下载中…";
    const text = await invoke("fetch_subscription", { url });
    $("configText").value = text;
    await invoke("import_config", { content: text });
    alert("订阅导入成功");
    loadProxies();
  } catch (e) {
    alert("订阅失败: " + e);
  } finally {
    $("sub").textContent = "订阅导入";
  }
});

$("restart").addEventListener("click", async () => {
  try {
    await invoke("restart_core_cmd");
    alert("核心已重启");
  } catch (e) {
    alert("重启失败: " + e);
  }
});

async function refreshCoreState() {
  try {
    const st = await invoke("core_status");
    coreState.textContent = st.running ? "核心运行中" : "核心未运行";
    coreState.className = "badge " + (st.running ? "ok" : "bad");
  } catch (e) {
    coreState.textContent = "状态未知";
    coreState.className = "badge bad";
  }
}

async function loadProxies() {
  try {
    const data = await invoke("list_proxies");
    const proxies = data.proxies || {};
    const groups = Object.entries(proxies).filter(
      ([, v]) => v && (v.type === "Selector" || v.type === "URLTest" || v.type === "Fallback")
    );
    if (groups.length === 0) {
      groupsEl.innerHTML = '<div class="hint">未找到代理组</div>';
      return;
    }
    groupsEl.innerHTML = "";
    for (const [name, info] of groups) {
      const row = document.createElement("div");
      row.className = "group";

      const label = document.createElement("span");
      label.className = "group-name";
      label.textContent = name;

      const select = document.createElement("select");
      const options = info.all || [];
      for (const opt of options) {
        const o = document.createElement("option");
        o.value = opt;
        o.textContent = opt;
        if (opt === info.now) o.selected = true;
        select.appendChild(o);
      }
      select.addEventListener("change", async () => {
        try {
          await invoke("select_proxy", { group: name, name: select.value });
        } catch (e) {
          console.error(e);
        }
      });

      row.appendChild(label);
      row.appendChild(select);
      groupsEl.appendChild(row);
    }
  } catch (e) {
    groupsEl.innerHTML = '<div class="hint">读取节点失败（核心可能未运行）</div>';
  }
}

invoke("navigate", { url: "https://www.bing.com" }).catch(console.error);
