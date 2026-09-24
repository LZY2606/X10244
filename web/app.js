"use strict";
const $ = (s) => document.querySelector(s);
let STATE = null, AT_SEQ = null;

async function api(path, opts) {
  const r = await fetch(path, opts);
  const j = await r.json();
  if (!r.ok) {
    if (j && j.error === "conflict") {
      alert(`并发冲突：你基于 seq ${j.expected} 编辑，当前已是 seq ${j.head_seq}（${j.head_event}）。\n界面已刷新，请在最新版本上重新提交（可解决的冲突，双方决定都保留）。`);
      await load();
    } else {
      alert("请求失败：" + (j.error || r.status));
    }
    throw new Error(j.error || "request failed");
  }
  return j;
}

async function postEvent(payload) {
  const base = STATE.head_seq;
  await api("/api/event", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ actor: "reviewer-web", payload, base_seq: base }),
  });
  await load();
}

async function load() {
  const qs = AT_SEQ ? `?at_seq=${AT_SEQ}` : "";
  STATE = await api("/api/state" + qs);
  render();
}

function currentChain() {
  const map = {};
  STATE.recomputes.forEach((r) => (map[r.old_id] = r.new_id));
  const cur = (id) => {
    let seen = new Set();
    while (map[id] && !seen.has(id)) { seen.add(id); id = map[id]; }
    return id;
  };
  return { map, cur };
}

function pointerSource(slot) {
  const p = STATE.pointers.find((p) => p.key === slot);
  return STATE.sources.find((s) => s.id === (p && p.source_id)) || null;
}

function derivedById(id) { return STATE.derived.find((d) => d.id === id); }

function currentDerived(kind) {
  const { cur } = currentChain();
  const list = STATE.derived.filter((d) => d.kind === kind);
  let best = null;
  list.forEach((d) => {
    if (!best || d.created_seq > best.created_seq) best = d;
  });
  if (best) best = derivedById(cur(best.id)) || best;
  return best;
}

function render() {
  $("#head-label").textContent = `head seq ${STATE.head_seq}` + (AT_SEQ ? ` · 时间旅行 @ ${AT_SEQ}` : "");
  renderTimeline();
  renderChart();
  renderDiagnostics();
  renderIntervals();
  renderDecisions();
}

function renderTimeline() {
  const el = $("#timeline");
  el.innerHTML = "";
  STATE.events.forEach((e) => {
    const d = document.createElement("div");
    d.className = "evt" + (e.seq === STATE.at_seq && !AT_SEQ ? " cur" : "") + (AT_SEQ && e.seq > AT_SEQ ? " past" : "");
    if (e.seq === (AT_SEQ || STATE.head_seq)) d.classList.add("cur");
    d.innerHTML = `<span class="seq">#${e.seq}</span><span class="kind">${e.kind}</span>
      <div class="muted pv">${(e.payload.slot || e.payload.derivation || e.payload.target || e.payload.key || "").toString().slice(0, 38)}</div>`;
    d.onclick = () => { AT_SEQ = AT_SEQ === e.seq ? null : e.seq; load(); };
    el.appendChild(d);
  });
}

function xs(v) { return $("#lg-logx").checked ? Math.log10(v) : v; }
function ys(v) { return $("#lg-logy").checked ? Math.log10(Math.max(v, 1e-6)) : v; }

function scaleFor(points) {
  const all = [];
  points.forEach((s) => s.data.forEach((p) => {
    if (isFinite(p[0]) && isFinite(p[1]) && p[1] > -1e30) all.push([p[0], p[1]]);
  }));
  if (!all.length) return null;
  let x0 = Math.min(...all.map((p) => p[0])), x1 = Math.max(...all.map((p) => p[0]));
  let y0 = Math.min(...all.map((p) => p[1])), y1 = Math.max(...all.map((p) => p[1]));
  if ($("#lg-logy").checked) { y0 = -3; y1 = Math.ceil(Math.log10(Math.max(...all.map((p) => Math.max(p[1], 1e-6))))); }
  if ($("#lg-logx").checked) { x0 = Math.floor(x0 * 10) / 10; x1 = Math.ceil(x1 * 10) / 10; }
  return { x0, x1, y0, y1 };
}

function renderChart() {
  const svg = $("#chart");
  svg.innerHTML = "";
  const W = 900, H = 420, M = { l: 56, r: 16, t: 12, b: 34 };
  const series = [];
  const sample = pointerSource("sample");
  const buf = pointerSource("buffer");
  if (sample && $("#lg-raw").checked)
    series.push({ label: "样品原始", cls: "raw-s", data: sample.payload.points.map((p, i) => [xs(p.q * (p.q_unit === "1/nm" ? 10 : 1)), ys(p.i)]) });
  if (buf && $("#lg-raw").checked)
    series.push({ label: "缓冲液当前", cls: "raw-b", data: buf.payload.points.map((p) => [xs(p.q * (p.q_unit === "1/nm" ? 10 : 1)) * 1, ys(p.i)]) });

  const ns = currentDerived("normalize");
  if (ns && $("#lg-norm").checked)
    series.push({ label: "样品归一化", cls: "norm", data: ns.payload.points.filter((p) => p.convertible).map((p) => [xs(p.q), ys(p.i)]) });
  const sub = currentDerived("subtract");
  if (sub && $("#lg-sub").checked) {
    const pts = sub.payload.points;
    series.push({ label: "扣背景", cls: "sub", data: pts.map((p) => [xs(p.q), ys(p.i)]), clickable: true, derived: sub });
    series.push({ label: "误差带 ±σ", cls: "band", data: pts.map((p) => [xs(p.q), ys(Math.max(p.i - p.sigma, 1e-9))]), bandLo: true, band: true, derived: sub });
    series.push({ label: "误差带 ±σ", cls: "band", data: pts.map((p) => [xs(p.q), ys(p.i + p.sigma)]), band: true, derived: sub });
  }
  if (!series.length) {
    svg.innerHTML = '<text x="400" y="210" text-anchor="middle" fill="#93a0b0">暂无数据（点击「重放案例包」）</text>';
    return;
  }
  const sc = scaleFor(series);
  const X = (x) => M.l + ((x - sc.x0) / (sc.x1 - sc.x0)) * (W - M.l - M.r);
  const Y = (y) => H - M.b - ((y - sc.y0) / (sc.y1 - sc.y0)) * (H - M.t - M.b);
  for (let i = 0; i <= 5; i++) {
    const yv = sc.y0 + (i / 5) * (sc.y1 - sc.y0);
    svg.innerHTML += `<line x1="${M.l}" x2="${W - M.r}" y1="${Y(yv)}" y2="${Y(yv)}" stroke="#eef1f5"/>
      <text x="${M.l - 6}" y="${Y(yv) + 3}" text-anchor="end" font-size="10" fill="#778496">${yv.toFixed(1)}</text>`;
  }
  for (let i = 0; i <= 6; i++) {
    const xv = sc.x0 + (i / 6) * (sc.x1 - sc.x0);
    svg.innerHTML += `<text x="${X(xv)}" y="${H - 12}" text-anchor="middle" font-size="10" fill="#778496">${xv.toFixed(2)}</text>`;
  }
  svg.innerHTML += `<text x="${M.l - 40}" y="14" font-size="11" fill="#44505f">${$("#lg-logy").checked ? "log10 I" : "I"}</text>
    <text x="${W - M.r - 30}" y="${H - 2}" font-size="11" fill="#44505f">${$("#lg-logx").checked ? "log10 q (1/Å)" : "q (1/Å)"}</text>`;
  const colors = { "raw-s": "#9aa7b8", "raw-b": "#c0a08a", norm: "#2f8f5b", sub: "#c0392b", band: "#e9a39b" };
  const bands = series.filter((s) => s.band);
  if (bands.length === 2) {
    let path = "";
    bands[0].data.forEach((p) => (path += `${X(p[0])},${Y(p[1])} `));
    bands[1].data.slice().reverse().forEach((p) => (path += `${X(p[0])},${Y(p[1])} `));
    svg.innerHTML += `<polygon points="${path}" fill="#c0392b" opacity="0.12"/>`;
  }
  series.filter((s) => !s.band).forEach((s) => {
    let d = "";
    s.data.forEach((p, i) => (d += (i ? "L" : "M") + X(p[0]).toFixed(1) + "," + Y(p[1]).toFixed(1) + " "));
    svg.innerHTML += `<path d="${d}" fill="none" stroke="${colors[s.cls] || "#888"}" stroke-width="${s.cls === "sub" ? 2 : 1.2}" opacity="${s.cls === "raw-s" ? 0.8 : 1}"/>`;
  });
  const sub2 = currentDerived("subtract");
  if (sub2 && $("#lg-sub").checked) {
    sub2.payload.points.forEach((p, i) => {
      svg.innerHTML += `<circle class="pthit" data-i="${i}" cx="${X(xs(p.q))}" cy="${Y(ys(p.i))}" r="7" fill="transparent"/>
        <circle data-i="${i}" cx="${X(xs(p.q))}" cy="${Y(ys(p.i))}" r="2.4" fill="${p.i < 0 ? "#8e44ad" : "#c0392b"}"/>`;
    });
    svg.querySelectorAll(".pthit").forEach((c) => {
      c.style.cursor = "pointer";
      c.onclick = () => showProvenance(sub2.id, +c.dataset.i);
    });
  }
}

function renderResiduals() {
  const svg = $("#reschart");
  svg.innerHTML = "";
  const sub = currentDerived("subtract");
  const W = 900, H = 130, M = { l: 56, r: 16, t: 10, b: 24 };
  if (!sub || !$("#lg-resid").checked) { svg.innerHTML = '<text x="400" y="60" text-anchor="middle" fill="#93a0b0" font-size="11">残差（无扣背景结果时为空）</text>'; return; }
  const pts = sub.payload.points;
  let x0 = xs(pts[0].q), x1 = xs(pts[pts.length - 1].q);
  let rmin = -4, rmax = 4;
  const X = (x) => M.l + ((x - x0) / (x1 - x0)) * (W - M.l - M.r);
  const Y = (r) => H - M.b - ((r - rmin) / (rmax - rmin)) * (H - M.t - M.b);
  [-2, 0, 2].forEach((r) => {
    svg.innerHTML += `<line x1="${M.l}" x2="${W - M.r}" y1="${Y(r)}" y2="${Y(r)}" stroke="${r === 0 ? "#b9c2cf" : "#eef1f5"}" stroke-dasharray="${r ? "3,3" : ""}"/>
      <text x="${M.l - 6}" y="${Y(r) + 3}" text-anchor="end" font-size="10" fill="#778496">${r}σ</text>`;
  });
  let d = "";
  pts.forEach((p, i) => { const r = p.i / p.sigma; d += (i ? "L" : "M") + X(xs(p.q)).toFixed(1) + "," + Y(Math.max(-4, Math.min(4, r))).toFixed(1) + " "; });
  svg.innerHTML += `<path d="${d}" fill="none" stroke="#2f6fed" stroke-width="1.4"/>
    <text x="${M.l}" y="12" font-size="11" fill="#44505f">标准化残差 I_sub/σ（含归一化与背景缩放不确定度） · 模式: ${sub.payload.mode}</text>`;
}

function allDiagnostics() {
  const out = [];
  STATE.derived.forEach((d) => (d.diagnostics || []).forEach((g) => out.push({ from: d.id, kind: d.kind, ...g })));
  return out;
}

function codeClass(code) {
  if (["no_overlap", "unit_unconvertible"].includes(code)) return "err";
  if (["q_grid_mismatch", "negative_intensity", "interval_overlap", "ambiguous_candidates",
       "interval_underdetermined", "guinier_nonphysical", "norm_factor_invalid"].includes(code)) return "warn";
  return "info";
}

function renderDiagnostics() {
  const el = $("#diagnostics");
  const ds = allDiagnostics();
  if (!ds.length) { el.innerHTML = '<p class="muted">无诊断。</p>'; return; }
  el.innerHTML = ds.map((g) =>
    `<span class="chip ${codeClass(g.code)}" title="来源派生 ${g.from}">${g.code}${g.point_index != null ? " #" + g.point_index : ""}</span>
     <div style="margin:0 0 5px 2px;color:#44505f">${g.message} <span class="muted pv">— ${g.from.slice(0, 26)}</span></div>`
  ).join("");
}

function renderIntervals() {
  renderResiduals();
  const el = $("#intervals");
  const { cur } = currentChain();
  const fits = STATE.derived.filter((d) => d.kind === "fit");
  let html = "";
  fits.forEach((f) => {
    html += `<div class="dec"><b>${f.id.slice(0, 30)}…</b> <span class="chip ${f.id === cur(f.id) ? "ok" : "warn"}">${f.validity.state}</span>
      <div class="muted pv">${f.validity.reason}</div>
      <table><tr><th>区间</th><th>参数</th><th>残差</th><th>n</th></tr>`;
    f.payload.fits.forEach((fit) => {
      const pp = fit.params.map(([k, v]) => `${k}=${v.toFixed(4)}`).join(", ");
      html += `<tr><td>${fit.kind} [${fit.q_min},${fit.q_max}]</td><td class="pv">${pp}</td><td>${fit.residual.toFixed(4)}</td><td>${fit.n_points}</td></tr>`;
    });
    html += `</table></div>`;
  });
  const subs = STATE.derived.filter((d) => d.kind === "subtract");
  if (subs.length) {
    html += `<h3 style="margin-top:8px">背景缩放候选（全部保留，不吞掉多解）</h3>`;
    subs.forEach((d) => {
      html += `<div class="dec"><span class="pv muted">${d.id.slice(0, 28)}… · scale=${d.params.scale}±${d.params.scale_sigma}</span>
        <table><tr><th>scale</th><th>高 q 残差 RMS</th><th>采纳</th></tr>`;
      d.candidates.forEach((c) => {
        html += `<tr><td>${c.scale.toFixed(4)}</td><td>${c.residual.toFixed(4)}</td><td>${c.adopted ? "★名义" : ""}</td></tr>`;
      });
      html += `</table></div>`;
    });
  }
  el.innerHTML = html || '<p class="muted">尚无区间拟合。</p>';
}

function renderDecisions() {
  const el = $("#decisions");
  if (!STATE.decisions.length) { el.innerHTML = '<p class="muted">尚无裁定。</p>'; return; }
  let html = "";
  STATE.decisions.forEach((d) => {
    html += `<div class="dec ${d.conflict ? "conflict" : ""} ${d.resolved_by ? "resolved" : ""}">
      <b>#${d.created_seq}</b> <span class="chip ${d.action === "accept" ? "ok" : "err"}">${d.action}</span>
      ${d.conflict ? '<span class="chip err">冲突</span>' : ""}
      ${d.resolved_by ? '<span class="chip info">已由 ' + d.resolved_by.slice(0, 18) + '… 解决</span>' : ""}
      <div>${d.note}</div><div class="muted pv">目标 ${d.target.slice(0, 34)}… · 基于 seq ${d.branch_base_seq}</div></div>`;
  });
  el.innerHTML = html;
}

async function showProvenance(id, pointIndex) {
  const p = await api("/api/provenance/" + encodeURIComponent(id));
  const el = $("#provenance");
  let html = `<div class="dec"><b>${p.kind}</b> <span class="pv muted">${p.id.slice(0, 24)}…</span>
    <div class="muted pv">${p.diagnostics.map((d) => d.code).join(", ") || "无诊断"}</div></div>
    <h3>两条（或全部）源曲线</h3>`;
  p.resolved_inputs.forEach((inp) => {
    const rec = inp.record;
    let pts = rec.type === "source" ? rec.payload.points : rec.payload.points;
    let conv = rec.type === "derived" ? pts.filter((q) => q.convertible !== false) : pts;
    if (rec.type === "source") conv = pts.map((q) => ({ q: q.q * (q.q_unit === "1/nm" ? 10 : 1), i: q.i }));
    html += `<div class="dec srcnode" data-srcid="${inp.id}"><b>${inp.slot}</b>: ${rec.type}/${rec.kind || ""} <span class="chip info">${inp.id.slice(0, 20)}…</span>`;
    if (rec.type === "source") {
      html += `<div class="muted">曝光 ${rec.payload.exposure_s}s · 厚度 ${rec.payload.thickness_cm}cm · 透射率 ${rec.payload.transmission}</div>`;
    }
    html += `<svg data-mini="${inp.slot}" width="320" height="90"></svg></div>`;
  });
  if (pointIndex != null && p.payload.points && p.payload.points[pointIndex]) {
    const dp = p.payload.points[pointIndex];
    html += `<div class="dec"><b>派生点 #${pointIndex}</b>
      <div>q=${dp.q.toFixed(5)} 1/Å · I_sub=${dp.i.toFixed(4)} ± ${dp.sigma.toFixed(4)}</div>
      <div class="muted pv">样品源点索引 ${dp.sample_index} · 缓冲液源点索引 ${dp.buffer_index ?? "—"}</div></div>`;
  }
  html += `<h3>裁定此派生</h3>
    <div class="row">
      <button id="d-accept">接受</button><button id="d-reject">否定</button>
      <label class="muted">调整 scale <input id="d-scale" type="number" step="0.01" value="${p.params.scale ?? 1}"></label>
      <label class="muted">scale σ <input id="d-ssigma" type="number" step="0.01" value="${p.params.scale_sigma ?? 0.03}"></label>
      <button id="d-adjust">按新参数重算</button>
    </div>`;
  el.innerHTML = html;
  p.resolved_inputs.forEach((inp) => drawMini(`svg[data-mini="${inp.slot}"]`, inp.record, pointIndex, p));
  $("#d-accept").onclick = () => postEvent({ target: p.id, action: "accept", note: "界面接受" });
  $("#d-reject").onclick = () => postEvent({ target: p.id, action: "reject", note: "界面否定" });
  $("#d-adjust").onclick = async () => {
    const params = { scale: +$("#d-scale").value, scale_sigma: +$("#d-ssigma").value };
    await postEvent({ derivation: "subtract", inputs: p.inputs, params });
    await postEvent({ target: deriveIdFrom(p.id), action: "accept", note: "参数调整后继续（旧版本保留）" }).catch(()=>{});
  };
}

function deriveIdFrom(_id) { return _id; }

function drawMini(sel, rec, highlightIndex, parent) {
  const svg = document.querySelector(sel);
  if (!svg) return;
  const W = 320, H = 90, M = 6;
  let pts;
  if (rec.type === "source") pts = rec.payload.points.map((q) => [q.q * (q.q_unit === "1/nm" ? 10 : 1), q.i]);
  else pts = rec.payload.points.filter((q) => q.convertible !== false).map((q) => [q.q, q.i]);
  pts = pts.filter((q) => isFinite(q[0]) && isFinite(q[1]));
  if (!pts.length) { svg.innerHTML = '<text x="10" y="45" font-size="10" fill="#93a0b0">无点（无重叠/未转换）</text>'; return; }
  const x0 = Math.min(...pts.map((q) => q[0])), x1 = Math.max(...pts.map((q) => q[0]));
  const pos = pts.map((q) => q[1]).filter((v) => v > 0);
  const y0 = 1e-4, y1 = Math.max(...pos) * 1.1;
  const X = (x) => M + ((x - x0) / (x1 - x0)) * (W - 2 * M);
  const Y = (y) => H - M - ((Math.log10(Math.max(y, y0)) - Math.log10(y0)) / (Math.log10(y1) - Math.log10(y0))) * (H - 2 * M);
  let d = "";
  pts.forEach((q, i) => (d += (i ? "L" : "M") + X(q[0]).toFixed(1) + "," + Y(q[1]).toFixed(1) + " "));
  svg.innerHTML = `<path d="${d}" fill="none" stroke="#2f6fed" stroke-width="1.2"/>`;
  const qp = parent.payload.points && highlightIndex != null ? parent.payload.points[highlightIndex] : null;
  if (qp) {
    const near = pts.map((q, i) => [i, Math.abs(q[0] - qp.q)]).sort((a, b) => a[1] - b[1])[0];
    const q = pts[near[0]];
    svg.innerHTML += `<circle cx="${X(q[0])}" cy="${Y(q[1])}" r="3.5" fill="#c0392b"/>`;
  }
}

$("#lg-raw").onchange = $("#lg-norm").onchange = $("#lg-sub").onchange =
  $("#lg-resid").onchange = $("#lg-logx").onchange = $("#lg-logy").onchange = () => render();

$("#btn-replay").onclick = async () => {
  const j = await api("/api/replay_case", { method: "POST" });
  AT_SEQ = null;
  alert(`已重放案例包：${j.replayed} 个事件（${j.case}）`);
  await load();
};
$("#btn-export").onclick = async () => {
  const j = await api("/api/export");
  const blob = new Blob([JSON.stringify(j, null, 1)], { type: "application/json" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob); a.download = "saxs-events.json"; a.click();
};
$("#file-import").onchange = async (e) => {
  const text = await e.target.files[0].text();
  await api("/api/import", { method: "POST", headers: { "content-type": "application/json" }, body: text });
  AT_SEQ = null; await load();
};
$("#btn-path").onclick = () => {
  const b = $("#path-banner");
  b.classList.toggle("hidden");
  b.innerHTML = `<b>90 秒样例路径</b><ol style="margin:6px 18px;padding:0">
    <li>点「重放案例包」→ 看原始 I(q)、归一化、扣背景+误差带、残差</li>
    <li>点扣除曲线任意点 → 右侧高亮样品与缓冲液两条源曲线、透射率/曝光/厚度与源点索引</li>
    <li>看诊断芯片：q_grid_mismatch、no_overlap、negative_intensity、interval_overlap、ambiguous_candidates</li>
    <li>点左侧旧事件 → 时间旅行恢复当时版本；再点一次返回最新</li>
    <li>右侧面板做接受/否定/调 scale；两人同版本编辑会得到可解决冲突</li></ol>`;
};

load();
