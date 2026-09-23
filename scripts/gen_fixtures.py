#!/usr/bin/env python3
"""生成 fixtures/case_pack 离线案例包（确定性输出，可重复运行）。"""
import json, math, os

OUT = os.path.join(os.path.dirname(__file__), "..", "fixtures", "case_pack")
os.makedirs(OUT, exist_ok=True)

def noise(i, seed):
    # 确定性伪噪声，范围约 [-1, 1]
    return math.sin(i * 12.9898 + seed * 78.233) * 0.8 + math.sin(i * 3.7 + seed) * 0.2

def dump(name, obj):
    with open(os.path.join(OUT, name), "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.write("\n")

# --- 样品曲线：Guinier 形态 I = I0*exp(-(q Rg)^2/3) + 平背景，q 单位 A^-1 ---
I0, RG, BKG = 1500.0, 20.0, 2.2
qs, i = [], 0
q = 0.008
while q <= 0.3001:
    qs.append(q)
    q *= 1.055
sample_pts = []
for i, qv in enumerate(qs):
    iv = I0 * math.exp(-((qv * RG) ** 2) / 3.0) + BKG + noise(i, 1.0) * 0.06
    ev = 0.02 * abs(iv) + 0.05
    sample_pts.append([round(qv, 6), round(iv, 5), round(ev, 5)])
dump("sample_curve.json", {
    "id": "curve.sample.raw",
    "kind": "raw_curve",
    "description": "溶菌酶样品 2D 积分后的 I(q)，探测器原始坐标",
    "q_unit": "A^-1", "i_unit": "a.u.",
    "points": sample_pts,
})

# --- 缓冲液 v1：与样品同网格（A^-1） ---
buf1 = []
for i, qv in enumerate(qs):
    iv = 2.29 + noise(i, 2.0) * 0.05
    buf1.append([round(qv, 6), round(iv, 5), 0.06])
dump("buffer_v1.json", {
    "id": "curve.buffer.v1", "kind": "buffer_candidate",
    "description": "缓冲液测量第 1 版（与样品同次曝光序列）",
    "q_unit": "A^-1", "i_unit": "a.u.", "points": buf1,
})

# --- 缓冲液 v2：错位网格，q 单位 nm^-1，只覆盖高 q 段（部分重叠） ---
buf2 = []
qv = 1.0
i = 0
while qv <= 3.2001:
    iv = 2.31 + noise(i, 3.0) * 0.05
    buf2.append([round(qv, 5), round(iv, 5), 0.06])
    qv += 0.097
    i += 1
dump("buffer_v2.json", {
    "id": "curve.buffer.v2", "kind": "buffer_candidate",
    "description": "缓冲液测量第 2 版（重新积分，nm^-1 网格，与样品错位）",
    "q_unit": "nm^-1", "i_unit": "a.u.", "points": buf2,
})

# --- 缓冲液 v3：q 区间与样品完全无重叠 ---
buf3 = []
qv = 3.5
i = 0
while qv <= 6.0001:
    buf3.append([round(qv, 5), round(2.4 + noise(i, 4.0) * 0.05, 5), 0.07])
    qv += 0.21
    i += 1
dump("buffer_v3.json", {
    "id": "curve.buffer.v3", "kind": "buffer_candidate",
    "description": "缓冲液测量第 3 版（广角段，q 区间与样品无重叠）",
    "q_unit": "nm^-1", "i_unit": "a.u.", "points": buf3,
})

# --- 样品元数据 ---
dump("sample_meta.json", {
    "id": "meta.sample",
    "exposure_s": 10.0,
    "thickness_mm": 1.5,
    "transmission": 0.42,
    "transmission_rel_unc": 0.024,
    "note": "透射率由二极管计数比给出，含 2.4% 相对不确定度",
})

# --- 仪器响应（单位不可换算，保留原值并附诊断） ---
resp = []
qv = 0.008
while qv <= 0.3001:
    resp.append([round(qv, 6), round(1.0 + 0.02 * math.sin(qv * 40.0), 5)])
    qv *= 1.055
dump("instrument_response.json", {
    "id": "inst.response", "kind": "instrument_response",
    "description": "探测器逐 q 效率响应",
    "q_unit": "A^-1", "value_unit": "counts/s",
    "factors": resp,
})

# --- 遮罩来源 ---
dump("mask.json", {
    "id": "mask.primary", "kind": "mask",
    "source": "mask_0427.xml",
    "q_unit": "A^-1",
    "intervals": [[0.282, 0.30]],
    "note": "束流阻挡器边缘遮罩，来自探测器掩膜文件",
})

# --- 重放脚本（机器可读的步骤清单，由引擎逐步执行） ---
dump("scenario.json", {
    "id": "scenario.replay",
    "title": "散射曲线台：SAXS 归一化、背景扣除与多区间模型证据",
    "steps": [
        {"id": "normalize", "label": "归一化：I/(T·d·t)，q 统一到 nm^-1"},
        {"id": "subtract_buf1", "label": "背景扣除：缓冲液 v1（同网格），缩放 0.96±0.02"},
        {"id": "fit_intervals", "label": "区间候选：Guinier×3（重叠）+ Porod×2，不自动合并"},
        {"id": "revise_transmission", "label": "参数修订：透射率 0.42 → 0.40，依赖图局部重算"},
        {"id": "introduce_buffer_v2", "label": "源数据局部改动：缓冲液 v2（错位网格，仅重叠区插值）"},
        {"id": "switch_buffer", "label": "缓冲液版本切换 v1→v2，两分支均保留"},
        {"id": "attempt_no_overlap", "label": "边界：缓冲液 v3 无重叠区，拒绝直接相减"},
        {"id": "conflicting_decisions", "label": "并发人工决定：同一版本上的冲突与分支保留"},
        {"id": "final_decision", "label": "最终裁定：采纳 G1，G2/G3 保留未采纳"},
    ],
})
print("fixtures written to", os.path.abspath(OUT))
