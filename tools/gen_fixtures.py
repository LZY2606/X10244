#!/usr/bin/env python3
"""生成 SAXS 专用离线案例包：物理合成曲线 + 缓冲版本 + q 网格错位变体。

内部 q 基准 1/A。样品 I_raw 对应 1.0 mm、T=0.87、60 s 的探测器计数；
缓冲另有版本与错位网格，供无重叠/版本切换/多候选残差情形重放。
"""
import json
import math
import os
import random

OUT = os.path.join(os.path.dirname(__file__), "..", "fixtures", "saxs-case")
os.makedirs(OUT, exist_ok=True)

random.seed(20260924)

I0 = 0.42          # cm^-1
RG = 16.0          # A（溶菌酶量级）
POROD_K = 5.0e-6   # cm^-1 A^4（高 q 尾，Beaucage 式统一公式接入）
DET_GAIN = 1.0e5   # 探测器增益：计数 = I_abs * 归一化常量 * gain
EXPOSURE = 60.0    # s
THICKNESS = 0.1    # cm
T_SAMPLE = 0.87
T_BUFFER = 0.92

def _logistic(x):
    return 1.0 / (1.0 + math.exp(-x))

# 样品绝对强度（绝对单位，1/cm）：Guinier + Beaucage 式高 q 尾。
def sample_i(q):
    guinier = I0 * math.exp(-(q * RG) ** 2 / 3.0)
    tail = POROD_K * q ** -4 * math.exp(-12.0 / (q * RG) ** 2)
    return guinier + tail

def write_curve(path, qs, intens_abs, thickness, transmission, *,
                q_unit="1/A", exposure=EXPOSURE, exp_unit="s",
                th_unit="cm", t_unit="t", noise=0.012, banner_extra=None,
                shift=0.0):
    lines = [
        "# SAXS radially integrated curve (case fixture)",
        f"# q_unit: {q_unit}",
        f"# I_unit: count",
        f"# exposure: {exposure} {exp_unit}",
        f"# thickness: {thickness} {th_unit}",
        f"# transmission: {transmission} {t_unit}",
    ]
    if banner_extra:
        lines.extend(banner_extra)
    lines.append("# q  I  dI")
    norm = exposure * thickness * transmission
    for i, q in enumerate(qs):
        qi = q + shift
        if qi <= 0:
            continue
        mean_count = intens_abs(q) * norm * DET_GAIN
        # 相对噪声 + 计数统计
        sig = abs(mean_count) * noise + math.sqrt(max(abs(mean_count), 0.0))
        val = random.gauss(mean_count, sig)
        lines.append(f"{qi:.6f}  {val:.5f}  {sig:.5f}")
    with open(path, "w") as fh:
        fh.write("\n".join(lines) + "\n")

# 主网格：0.008 .. 0.32 1/A，对数等距 60 点
qs = [10 ** (-2.09691 + i * (math.log10(0.32) - -2.09691) / 59) for i in range(60)]
write_curve(os.path.join(OUT, "sample_lysozyme_v1.dat"), qs, sample_i,
            THICKNESS, T_SAMPLE)

# 缓冲：低 q 阻尼（q^4 形式）的尾巴 + 小平背景，保证任何有限 q 都不发散。
def buffer_i(q):
    tail = 2.5e-7 * q ** -4 * math.exp(-(0.045 / q) ** 2)
    return 0.0006 + tail
write_curve(os.path.join(OUT, "buffer_v1.dat"), qs, buffer_i,
            THICKNESS, T_BUFFER)

# 缓冲 v2：平背景微调 0.004 -> 0.0048，尾项微调（参数修订）
def buffer_i_v2(q):
    tail = 2.3e-7 * q ** -4 * math.exp(-(0.045 / q) ** 2)
    return 0.00068 + tail
write_curve(os.path.join(OUT, "buffer_v2.dat"), qs, buffer_i_v2,
            THICKNESS, T_BUFFER)

# q 网格错位缓冲：同样范围但不同点（对数等距 46 点，轻微整体偏移）
qs_shift = [10 ** (-2.05 + i * (math.log10(0.30) - -2.05) / 45) for i in range(46)]
write_curve(os.path.join(OUT, "buffer_shifted_grid.dat"), qs_shift, buffer_i,
            THICKNESS, T_BUFFER)

# 无重叠缓冲：q 全部落在样品高 q 段之外（更高 q）
qs_far = [0.45 + i * 0.01 for i in range(20)]
write_curve(os.path.join(OUT, "buffer_no_overlap.dat"), qs_far, buffer_i,
            THICKNESS, T_BUFFER)

# 仪器响应（q, factor），全范围缓变
with open(os.path.join(OUT, "instrument_response_v1.dat"), "w") as fh:
    fh.write("# instrument response (q 1/A, relative factor)\n# q  factor\n")
    for i in range(25):
        q = 10 ** (-2.1 + i * (math.log10(0.35) + 2.1) / 24)
        f = 0.97 + 0.05 * (i / 24.0)
        fh.write(f"{q:.5f}  {f:.5f}\n")

# 遮罩：排除两个束流停止点与一个坏点（按样品 v1 点索引）
with open(os.path.join(OUT, "mask_v1.dat"), "w") as fh:
    fh.write("# mask: excluded raw point indices\n0\n1\n37\n")
with open(os.path.join(OUT, "mask_v2.dat"), "w") as fh:
    fh.write("# mask v2: one additional bad pixel cluster at index 38\n0\n1\n37\n38\n")

# 故意放入无法转换的透射率表示，验证“留在原位置并附诊断”
manifest = {
    "case_id": "saxs-lysozyme-2026-09",
    "title": "散射曲线台：SAXS 归一化、背景扣除与多区间模型证据",
    "materials": {
        "sample": {
            "file": "sample_lysozyme_v1.dat",
            "label": "lysozyme 4 mg/mL",
            "transmission_override": None
        },
        "buffers": [
            {"id": "buffer-v1", "file": "buffer_v1.dat", "label": "缓冲液批次 A（记录于测量当日）"},
            {"id": "buffer-v2", "file": "buffer_v2.dat", "label": "缓冲液批次 A 复核版（平背景修订）"},
            {"id": "buffer-shifted", "file": "buffer_shifted_grid.dat", "label": "不同 q 网格的缓冲积分"},
            {"id": "buffer-no-overlap", "file": "buffer_no_overlap.dat", "label": "无重叠 q 区间的缓冲积分"}
        ]
    },
    "instrument_response": {"id": "resp-v1", "file": "instrument_response_v1.dat"},
    "masks": [
        {"id": "mask-v1", "file": "mask_v1.dat"},
        {"id": "mask-v2", "file": "mask_v2.dat"}
    ],
    "norm_rel_sigma": 0.02,
    "scale_uncertainty_threshold": 0.15,
    "unconvertible_examples": {
        "transmission_raw": "n/a",
        "transmission_unit": "t",
        "note": "导入演练时把样品透射率替换为该值：不可转换，留在原位置并附诊断，不画图丢弃"
    }
}
with open(os.path.join(OUT, "manifest.json"), "w") as fh:
    json.dump(manifest, fh, ensure_ascii=False, indent=2)

print("fixtures written to", OUT)
