#!/usr/bin/env python3
"""生成 SAXS 散射曲线台离线案例包 fixtures/case_pack.json。

数据模型（合成但领域真实）：
- 样品：稀溶液溶菌酶，I(q) = 缓冲液本底 + I0*exp(-(q*Rg)^2/3) + A/q^4，Rg=25A
- 缓冲液 v1：同 q 网格；v2：q 网格偏移且以 1/nm 记录（含一个无法转换单位的点）;
- vv3：q 范围与样品完全错开（无重叠区）
- 仪器响应与遮罩作为独立来源保留
内容寻址 id 的计算与 Rust 端 src/ids.rs 完全一致。
"""
import hashlib, json, math, pathlib

def cid(kind, payload):
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    h = hashlib.sha256(kind.encode() + b"\0" + canonical.encode()).hexdigest()[:24]
    return f"{kind}-{h}"

def r6(x):
    return round(x, 6)

Q_N = 100
q_grid = [r6(i * 0.005) for i in range(1, Q_N + 1)]  # 0.005 .. 0.500 1/A

def noise(i, amp):
    return amp * math.sin(i * 1.7) * math.cos(i * 0.31)

def buffer_level(q, i):
    return 0.85 + 0.02 * math.sin(q * 30.0) + noise(i, 0.04)

def sample_level(q, i):
    rg = 25.0
    i0 = 3000.0
    protein = i0 * math.exp(-((q * rg) ** 2) / 3.0) + 0.000405 / (q ** 4)
    return buffer_level(q, i) + protein + noise(i + 500, 0.08)

def sigma_of(i_val):
    return r6(0.02 * abs(i_val) + 0.05)

def curve(points, exposure, thickness, transmission, desc):
    return {
        "points": points,
        "exposure_s": exposure,
        "thickness_cm": thickness,
        "transmission": transmission,
        "description": desc,
    }

sample_pts = [
    {"q": q, "q_unit": "1/A", "i": r6(sample_level(q, i)), "sigma": sigma_of(sample_level(q, i))}
    for i, q in enumerate(q_grid)
]
buffer_v1_pts = [
    {"q": q, "q_unit": "1/A", "i": r6(buffer_level(q, i)), "sigma": sigma_of(buffer_level(q, i))}
    for i, q in enumerate(q_grid)
]
# v2：q 网格偏移 0.0004，且以 1/nm 记录；最后一个点单位无法转换（保留原值+诊断）
buffer_v2_pts = []
for i, q in enumerate(q_grid):
    q_shift = r6((q + 0.0004) / 10.0)  # 1/nm
    lvl = buffer_level(q, i) + 0.01
    buffer_v2_pts.append({"q": q_shift, "q_unit": "1/nm", "i": r6(lvl), "sigma": sigma_of(lvl)})
buffer_v2_pts.append({"q": 512.0, "q_unit": "detector-pixel", "i": 0.9, "sigma": 0.07})
# v3：q 范围 0.6..0.9，与样品无重叠
buffer_v3_pts = [
    {"q": r6(0.6 + i * 0.01), "q_unit": "1/A", "i": r6(0.8 + noise(i, 0.03)), "sigma": 0.06}
    for i in range(31)
]

sample_body = curve(sample_pts, 2.0, 0.15, 0.31, "溶菌酶 4.2 mg/mL，二维积分后 I(q)")
buffer_v1_body = curve(buffer_v1_pts, 2.0, 0.15, 0.33, "缓冲液 20mM HEPES pH7.5，第 1 版")
buffer_v2_body = curve(buffer_v2_pts, 2.0, 0.15, 0.33, "缓冲液第 2 版：q 网格偏移且以 1/nm 记录")
buffer_v3_body = curve(buffer_v3_pts, 2.0, 0.15, 0.33, "缓冲液第 3 版：q 范围与样品错开")
buffer_v4_body = curve(buffer_v1_pts, 2.0, 0.15, 0.34, "缓冲液第 4 版：透射率订正 0.33→0.34")

instrument_body = {
    "kind": "instrument_response",
    "detector": "PILATUS3 2M",
    "wavelength_A": 1.0,
    "efficiency_curve": [[0.005, 0.981], [0.1, 0.988], [0.3, 0.993], [0.5, 0.995]],
    "note": "探测器效率与波长，来自 2026-09 束线标定",
}
mask_body = {
    "kind": "mask",
    "source_file": "mask-2026-09-10.edf",
    "masked_q_ranges": [[0.0, 0.004]],
    "beamstop_shadow": True,
    "note": "束流阻挡器阴影与坏像素遮罩",
}

def src_id(slot, kind, version, body):
    return cid("src", {"slot": slot, "kind": kind, "version": version, "body": body})

def der_id(kind, inputs, params):
    return cid("der", {"kind": kind, "inputs": inputs, "params": params})

sid_sample = src_id("sample", "sample", 1, sample_body)
sid_buf1 = src_id("buffer", "buffer", 1, buffer_v1_body)
sid_buf2 = src_id("buffer", "buffer", 2, buffer_v2_body)
sid_buf3 = src_id("buffer", "buffer", 3, buffer_v3_body)

norm_params = {"norm_uncertainty": 0.02}
der_norm_sample = der_id("normalize", {"curve": sid_sample}, norm_params)
der_norm_buf1 = der_id("normalize", {"curve": sid_buf1}, norm_params)

sub_params_1 = {"scale": 1.0, "scale_sigma": 0.03}
sub_params_2 = {"scale": 1.05, "scale_sigma": 0.03}
der_sub_1 = der_id("subtract", {"sample": der_norm_sample, "buffer": der_norm_buf1}, sub_params_1)
der_sub_2 = der_id("subtract", {"sample": der_norm_sample, "buffer": der_norm_buf1}, sub_params_2)

fit_intervals_plain = [
    {"kind": "guinier", "q_min": 0.006, "q_max": 0.02},
    {"kind": "porod", "q_min": 0.15, "q_max": 0.45},
]
fit_intervals_overlap = [
    {"kind": "guinier", "q_min": 0.006, "q_max": 0.03},
    {"kind": "porod", "q_min": 0.025, "q_max": 0.45},
]
der_fit_1 = der_id("fit", {"curve": der_sub_1}, {"intervals": fit_intervals_plain})
der_fit_2 = der_id("fit", {"curve": der_sub_1}, {"intervals": fit_intervals_overlap})

dec_accept = cid("dec", {"target": der_sub_1, "action": "accept", "note": "残差尾部平稳，采纳名义缩放", "seq": 14})
dec_reject = cid("dec", {"target": der_sub_1, "action": "reject", "note": "高 q 负强度偏多，怀疑缓冲液缩放", "seq": 15})

events = [
    {"actor": "beamline", "kind": "import_source", "payload": {"slot": "sample", "source_kind": "sample", "body": sample_body}},
    {"actor": "beamline", "kind": "import_source", "payload": {"slot": "buffer", "source_kind": "buffer", "body": buffer_v1_body}},
    {"actor": "beamline", "kind": "import_source", "payload": {"slot": "buffer", "source_kind": "buffer", "body": buffer_v2_body}},
    {"actor": "beamline", "kind": "import_source", "payload": {"slot": "buffer", "source_kind": "buffer", "body": buffer_v3_body}},
    {"actor": "beamline", "kind": "import_source", "payload": {"slot": "instrument_response", "source_kind": "instrument_response", "body": instrument_body}},
    {"actor": "beamline", "kind": "import_source", "payload": {"slot": "mask", "source_kind": "mask", "body": mask_body}},
    {"actor": "alice", "kind": "derive", "payload": {"derivation": "normalize", "inputs": {"curve": sid_sample}, "params": norm_params}},
    {"actor": "alice", "kind": "derive", "payload": {"derivation": "normalize", "inputs": {"curve": sid_buf1}, "params": norm_params}},
    {"actor": "alice", "kind": "derive", "payload": {"derivation": "subtract", "inputs": {"sample": der_norm_sample, "buffer": der_norm_buf1}, "params": sub_params_1}},
    {"actor": "alice", "kind": "derive", "payload": {"derivation": "fit", "inputs": {"curve": der_sub_1}, "params": {"intervals": fit_intervals_plain}}},
    {"actor": "alice", "kind": "derive", "payload": {"derivation": "fit", "inputs": {"curve": der_sub_1}, "params": {"intervals": fit_intervals_overlap}}},
    {"actor": "alice", "kind": "derive", "payload": {"derivation": "subtract", "inputs": {"sample": der_norm_sample, "buffer": der_norm_buf1}, "params": sub_params_2}},
    {"actor": "alice", "kind": "decide", "payload": {"target": der_sub_1, "action": "accept", "note": "残差尾部平稳，采纳名义缩放"}},
    {"actor": "bob", "kind": "decide", "payload": {"target": der_sub_1, "action": "reject", "note": "高 q 负强度偏多，怀疑缓冲液缩放"}},
    {"actor": "alice", "kind": "decide", "payload": {"target": der_sub_1, "action": "accept", "note": "复核：负强度点在 2σ 内，维持采纳", "resolves": dec_reject}},
    {"actor": "bob", "kind": "set_pointer", "payload": {"key": "buffer", "source_id": sid_buf2}},
    {"actor": "bob", "kind": "set_pointer", "payload": {"key": "buffer", "source_id": sid_buf3}},
    {"actor": "bob", "kind": "set_pointer", "payload": {"key": "buffer", "source_id": sid_buf1}},
    {"actor": "beamline", "kind": "source_revision", "payload": {"slot": "buffer", "source_kind": "buffer", "body": buffer_v4_body}},
]

pack = {
    "title": "散射曲线台：SAXS 归一化、背景扣除与多区间模型证据",
    "narrative": "小角散射小组导入二维积分后的 I(q)、透射率、曝光和缓冲液记录，生成可审阅的归一化与背景扣除链；"
                 "随后触发 q 网格错位、无重叠区、背景缩放不确定度、负强度、区间重叠、缓冲液版本切换与多候选残差，"
                 "并包含一次参数修订、一次局部源数据改动与一次互相冲突的人工决定。",
    "expected_identities": {
        "sample": sid_sample, "buffer_v1": sid_buf1, "buffer_v2": sid_buf2, "buffer_v3": sid_buf3,
        "norm_sample": der_norm_sample, "norm_buffer": der_norm_buf1,
        "subtract_nominal": der_sub_1, "subtract_rescaled": der_sub_2,
        "fit_plain": der_fit_1, "fit_overlap": der_fit_2,
    },
    "events": events,
}
out = pathlib.Path(__file__).parent / "case_pack.json"
out.write_text(json.dumps(pack, ensure_ascii=False, indent=1))
print("wrote", out, "events:", len(events))
for k, v in pack["expected_identities"].items():
    print(f"  {k}: {v}")
