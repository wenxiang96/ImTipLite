const { invoke } = window.__TAURI__.core;

const checkboxes = ["paused", "follow_mouse", "ignore_fullscreen", "show_on_change", "transparent_bg"];
const sliders = {
  timeout: ["timeout_v", "float"],
  poll_interval: ["poll_v", "int"],
  offset_x: ["ox_v", "float"],
  offset_y: ["oy_v", "float"],
  alpha: ["alpha_v", "int"],
  font_size: ["font_size_v", "int"],
  cell_w: ["cell_w_v", "int"],
  height: ["height_v", "int"],
  radius: ["radius_v", "int"],
  border_width: ["border_width_v", "int"],
};
const texts = [];
const FONT_SELECT_ID = "font_name";
const DEFAULT_FONT = "Microsoft YaHei UI";
const colors = ["color_bg", "color_border", "color_cn", "color_en", "color_full", "color_half", "color_caps"];
const chars = ["char_cn", "char_en", "char_full", "char_half", "char_caps", "char_other"];
const previewIds = ["pv_full", "pv_half", "pv_caps"];

let loaded = false;   // loadSettings 完成前, 不响应任何保存/预览
let saveTimer = 0;
let previewTimer = 0;
let previewRevision = 0;
let autoTimer = 0;

function gather() {
  const cfg = {};
  for (const k of checkboxes) cfg[k] = document.getElementById(k).checked;
  for (const [k, [, type]] of Object.entries(sliders)) {
    const value = document.getElementById(k).value;
    cfg[k] = type === "float" ? Number.parseFloat(value) : Number.parseInt(value, 10);
  }
  for (const k of texts) cfg[k] = document.getElementById(k).value;
  // 字体下拉可能尚未填充(loadSettings 里异步 invoke 期间): 此时不能把空字符串写回配置,
  // 否则会覆盖用户已保存的字体(表现为"打开设置后字体被重置为默认")。
  const fontEl = document.getElementById(FONT_SELECT_ID);
  const fontVal = fontEl ? fontEl.value : "";
  if (fontVal) cfg[FONT_SELECT_ID] = fontVal;
  for (const k of colors) cfg[k] = document.getElementById(k).value;
  for (const k of chars) cfg[k] = document.getElementById(k).value;
  return cfg;
}

function updateSliderLabel(key, value) {
  const label = sliders[key]?.[0];
  if (label) document.getElementById(label).textContent = value;
}

function schedulePreview() {
  if (!loaded) return;
  clearTimeout(previewTimer);
  const revision = ++previewRevision;
  previewTimer = setTimeout(async () => {
    try {
      const urls = await invoke("tip_preview", { cfg: gather() });
      // Ignore an older render if the user changed controls while it was in flight.
      if (revision !== previewRevision || !Array.isArray(urls)) return;
      previewIds.forEach((id, i) => {
        const img = document.getElementById(id);
        if (img && urls[i]) img.src = urls[i];
      });
    } catch (error) {
      console.error("提示预览渲染失败", error);
    }
  }, 70);
}

function scheduleSave() {
  if (!loaded) return;
  const status = document.getElementById("saved");
  status.textContent = "保存中…";
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      await invoke("set_config", { cfg: gather() });
      status.textContent = "设置已保存并生效";
      setTimeout(() => {
        if (status.textContent === "设置已保存并生效") status.textContent = "";
      }, 1600);
    } catch (error) {
      status.textContent = "保存失败";
      console.error("设置保存失败", error);
    }
  }, 220);
}

// 用系统已安装字体填充下拉框。找不到当前配置的字体时临时补一项, 避免显示空白。
async function populateFonts(current) {
  const sel = document.getElementById(FONT_SELECT_ID);
  if (!sel) return;
  let fonts = [];
  try {
    fonts = await invoke("list_fonts");
  } catch (error) {
    console.error("读取系统字体失败", error);
  }
  if (!Array.isArray(fonts) || fonts.length === 0) {
    // 后端枚举失败: 至少保留当前值, 不让下拉框空着
    fonts = current ? [current] : [DEFAULT_FONT];
  }
  // 常用字体排在最前, 其余按名称排序(后端已排序)
  const preferred = [DEFAULT_FONT, "微软雅黑", "SimHei", "SimSun", "KaiTi", "Segoe UI", "Consolas", "Arial"];
  const head = preferred.filter((f) => fonts.some((x) => x.toLowerCase() === f.toLowerCase()));
  const headSet = new Set(head.map((f) => f.toLowerCase()));
  const tail = fonts.filter((f) => !headSet.has(f.toLowerCase()));

  sel.innerHTML = "";
  const addGroup = (label, items) => {
    if (!items.length) return;
    const group = document.createElement("optgroup");
    group.label = label;
    for (const name of items) {
      const opt = document.createElement("option");
      opt.value = name;
      opt.textContent = name;
      group.appendChild(opt);
    }
    sel.appendChild(group);
  };
  addGroup("常用", head);
  addGroup("全部字体", tail);

  // 当前配置的字体若不在列表里, 补一项并选中(例如刚卸载过的字体)
  const want = current || DEFAULT_FONT;
  if (!Array.from(sel.options).some((o) => o.value.toLowerCase() === want.toLowerCase())) {
    const opt = document.createElement("option");
    opt.value = want;
    opt.textContent = `${want} (未安装)`;
    sel.insertBefore(opt, sel.firstChild);
  }
  sel.value = Array.from(sel.options).find((o) => o.value.toLowerCase() === want.toLowerCase()).value;
}

function bindSettings() {
  const onChange = (key, isColor = false) => (event) => {
    updateSliderLabel(key, event.target.value);
    if (isColor) {
      const hex = document.getElementById(`hex_${key}`);
      if (hex) hex.textContent = event.target.value.toUpperCase();
    }
    schedulePreview();
    scheduleSave();
  };

  for (const key of checkboxes) {
    document.getElementById(key)?.addEventListener("change", onChange(key));
  }
  for (const key of Object.keys(sliders)) {
    document.getElementById(key)?.addEventListener("input", onChange(key));
  }
  for (const key of texts) {
    document.getElementById(key)?.addEventListener("input", onChange(key));
  }
  document.getElementById(FONT_SELECT_ID)?.addEventListener("change", onChange(FONT_SELECT_ID));
  for (const key of colors) {
    document.getElementById(key)?.addEventListener("input", onChange(key, true));
  }
  for (const key of chars) {
    document.getElementById(key)?.addEventListener("input", onChange(key));
  }
}

async function loadSettings() {
  const cfg = await invoke("get_config");
  for (const key of checkboxes) {
    const el = document.getElementById(key);
    if (el) el.checked = Boolean(cfg[key]);
  }
  for (const [key, [label]] of Object.entries(sliders)) {
    const el = document.getElementById(key);
    if (!el) continue;
    el.value = cfg[key];
    updateSliderLabel(key, cfg[key]);
  }
  for (const key of texts) {
    const el = document.getElementById(key);
    if (el) el.value = cfg[key];
  }
  await populateFonts(cfg[FONT_SELECT_ID]);
  loaded = true;   // 字体下拉填充完毕, 之后才允许回写
  for (const key of colors) {
    const el = document.getElementById(key);
    if (el) el.value = String(cfg[key]).toLowerCase();
    const hex = document.getElementById(`hex_${key}`);
    if (hex) hex.textContent = String(cfg[key]).toUpperCase();
  }
  for (const key of chars) {
    const el = document.getElementById(key);
    if (el) el.value = cfg[key];
  }

  const autostart = document.getElementById("autostart");
  if (autostart) {
    try { autostart.checked = Boolean(await invoke("get_autostart")); }
    catch (error) { console.error("读取开机自启状态失败", error); }
  }
  schedulePreview();
}

function bindAutostart() {
  const checkbox = document.getElementById("autostart");
  if (!checkbox) return;
  checkbox.addEventListener("change", async () => {
    clearTimeout(autoTimer);
    const requested = checkbox.checked;
    checkbox.disabled = true;
    try {
      const ok = await invoke("set_autostart", { enabled: requested });
      if (!ok) throw new Error("注册表更新失败");
      const status = document.getElementById("saved");
      status.textContent = requested ? "已启用开机自启" : "已关闭开机自启";
      autoTimer = setTimeout(() => { status.textContent = ""; }, 1600);
    } catch (error) {
      checkbox.checked = !requested;
      document.getElementById("saved").textContent = "开机自启设置失败";
      console.error(error);
    } finally {
      checkbox.disabled = false;
    }
  });
}

window.addEventListener("DOMContentLoaded", async () => {
  bindSettings();
  bindAutostart();
  document.getElementById("reset")?.addEventListener("click", async () => {
    try {
      loaded = false;
      await invoke("reset_config");
      await loadSettings();
      document.getElementById("saved").textContent = "已恢复默认设置";
    } catch (error) {
      document.getElementById("saved").textContent = "恢复默认失败";
      console.error(error);
    }
  });
  try {
    await loadSettings();
  } catch (error) {
    document.getElementById("saved").textContent = "设置加载失败";
    console.error("设置加载失败", error);
  }
});
