#!/usr/bin/env python3
"""生成 SAXS 案例包 fixture（离线、确定性）。测试只读取生成产物。"""
import json, math, os

OUT = os.path.join(os.path.dirname(__file__), "case_pack")
os.makedirs(OUT, exist_ok=True)

def logspace(a, b, n):
    return [a * (b / a) ** (k / (n - 1)) for k in range(n)]

def write_dat(path, q, i, err, header):
    with open(path, "w") as f:
        f.write(header)
        for a, b, c in zip(q, i, err):
            f.write(f"{a:.6e} {b:.6e} {c:.6e}\n")

# ---- 样品：溶菌酶 5 mg/mL，Rg≈25 Å 的 Guinier 形 + 恒定背景 0.02 ----
q_s = logspace(0.010, 0.50, 120)
i_s = [math.exp(-((q * 25.0) ** 2) / 3.0) + 0.020 for q in q_s]
e_s = [0.03 * v + 5e-4 for v in i_s]
write_dat(
    os.path.join(OUT, "sample_a1.dat"), q_s, i_s, e_s,
    "# sample: lysozyme_5mgml_A1\n# q_unit: 1/angstrom\n# i_unit: 1/cm\n"
    "# columns: q I(q) sigma_I\n# exposure_s: 1.0\n",
)

# ---- 缓冲液 v1：同 q 网格，可直接逐点相减 ----
i_b1 = [0.018 * (1.0 + 0.10 * math.exp(-q / 0.05)) for q in q_s]
e_b1 = [0.03 * v + 3e-4 for v in i_b1]
write_dat(
    os.path.join(OUT, "buffer_b1_v1.dat"), q_s, i_b1, e_b1,
    "# buffer: B1, version v1 (same q grid as sample)\n# q_unit: 1/angstrom\n"
    "# i_unit: 1/cm\n# columns: q I(q) sigma_I\n",
)

# ---- 缓冲液 v2：网格错位（不同点数/起点），高 q 平台高于样品 -> 扣后负强度 ----
q_b2 = logspace(0.015, 0.52, 100)
i_b2 = [0.024 * (1.0 + 0.08 * math.exp(-q / 0.04)) for q in q_b2]
e_b2 = [0.03 * v + 3e-4 for v in i_b2]
write_dat(
    os.path.join(OUT, "buffer_b1_v2.dat"), q_b2, i_b2, e_b2,
    "# buffer: B1, version v2 (re-integrated, shifted q grid)\n# q_unit: 1/angstrom\n"
    "# i_unit: 1/cm\n# columns: q I(q) sigma_I\n",
)

# ---- 无重叠缓冲液：q 区间完全在样品之外 ----
q_no = logspace(0.60, 1.00, 30)
i_no = [0.015 for _ in q_no]
e_no = [0.03 * v + 3e-4 for v in i_no]
write_dat(
    os.path.join(OUT, "buffer_no_overlap.dat"), q_no, i_no, e_no,
    "# buffer: B1, wrong-q-range export (no overlap with sample)\n"
    "# q_unit: 1/angstrom\n# i_unit: 1/cm\n# columns: q I(q) sigma_I\n",
)

# ---- 元数据：厚度用 cm（派生层才换算 mm），透射率带不确定度 ----
meta = {
    "sample_id": "lysozyme_5mgml_A1",
    "exposure": {"value": 1.0, "unit": "s"},
    "thickness": {"value": 0.15, "unit": "cm"},
    "transmission": {"value": 0.32, "unit": "1", "uncertainty": 0.01},
    "background_scale": {"value": 1.0, "unit": "1", "uncertainty": 0.02},
    "instrument_response_file": "instrument_response.json",
    "mask_file": "mask.json",
}
json.dump(meta, open(os.path.join(OUT, "metadata.json"), "w"), indent=2, ensure_ascii=False)

# ---- 每个缓冲液各自的曝光/透射率（局部源改动只影响对应分支）----
bmeta = {
    "v1": {"exposure": {"value": 1.0, "unit": "s"},
           "transmission": {"value": 0.34, "unit": "1", "uncertainty": 0.01}},
    "v2": {"exposure": {"value": 2.0, "unit": "s"},
           "transmission": {"value": 0.33, "unit": "1", "uncertainty": 0.01}},
    "no_overlap": {"exposure": {"value": 1.0, "unit": "s"},
                   "transmission": {"value": 0.34, "unit": "1", "uncertainty": 0.01}},
}
json.dump(bmeta, open(os.path.join(OUT, "buffer_meta.json"), "w"), indent=2, ensure_ascii=False)

# ---- 仪器响应：任意单位，不可换算为绝对强度 -> 保留原位并附诊断 ----
inst = {
    "unit": "arb",
    "coordinate_basis": "detector_pixel",
    "description": "per-pixel flat-field response, arbitrary units; NOT convertible to absolute intensity",
    "values": [round(1.0 + 0.02 * math.sin(k / 7.0), 6) for k in range(16)],
}
json.dump(inst, open(os.path.join(OUT, "instrument_response.json"), "w"), indent=2, ensure_ascii=False)

# ---- 遮罩来源 ----
mask = {
    "source": "detector_edge+beamstop+flight_path",
    "coordinate_basis": "detector_pixel",
    "rects": [[0, 0, 8, 512], [504, 0, 512, 512], [240, 240, 272, 272]],
}
json.dump(mask, open(os.path.join(OUT, "mask.json"), "w"), indent=2, ensure_ascii=False)

# ---- 重放脚本：按序经过所有边界组合 ----
replay = {
    "name": "散射曲线台：SAXS 归一化、背景扣除与多区间模型证据",
    "steps": [
        {"op": "import_baseline", "note": "原始 q/I/误差、曝光、厚度、透射率、缓冲液候选、仪器响应、遮罩来源全部保留"},
        {"op": "subtract", "buffer": "v1", "note": "同网格直接相减基线"},
        {"op": "subtract", "buffer": "v2", "note": "q 网格错位 -> 仅重叠区插值；高 q 负强度保留"},
        {"op": "subtract", "buffer": "no_overlap", "note": "无重叠区 -> 状态 undetermined，不生成假曲线"},
        {"op": "revise_param", "param": "background_scale", "value": 1.05, "uncertainty": 0.03,
         "note": "参数修订：背景缩放及其不确定度进入误差"},
        {"op": "revise_source", "node": "buffer_raw:v2", "field": "transmission", "value": 0.30,
         "note": "局部源改动：只重算 v2 分支，其余保持有效并说明理由"},
        {"op": "switch_buffer", "to": "v2", "note": "缓冲液版本切换：未选分支保留"},
        {"op": "fit_intervals", "guinier": [0.010, 0.060], "porod": [0.040, 0.500],
         "note": "区间重叠 -> Guinier 与 Porod 不自动合并；多候选残差全部保留"},
        {"op": "conflict", "node": "review_state", "users": ["alice", "bao"],
         "note": "两名审阅者从同一版本作出互相冲突的决定"},
    ],
}
json.dump(replay, open(os.path.join(OUT, "replay.json"), "w"), indent=2, ensure_ascii=False)
print("case pack generated:", sorted(os.listdir(OUT)))
