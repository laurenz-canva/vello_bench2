# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "matplotlib>=3.9,<4",
# ]
# ///

"""Generate charts from the external-texture benchmark CSV files.

Run with:
    uv run external_texture_bench/plot_benchmark_results.py
"""

from __future__ import annotations

import argparse
import csv
import math
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
from matplotlib.lines import Line2D  # noqa: E402


SCRIPT_DIR = Path(__file__).resolve().parent


@dataclass(frozen=True)
class BenchmarkPoint:
    rect_count: int
    image_fps: float
    external_fps: float
    image_cpu_ms: float
    external_cpu_ms: float


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate external-texture benchmark charts."
    )
    parser.add_argument(
        "--benchmarks",
        type=Path,
        default=SCRIPT_DIR / "benchmark_results.csv",
        help="Benchmark results CSV.",
    )
    parser.add_argument(
        "--gpus",
        type=Path,
        default=SCRIPT_DIR / "device_gpu_scores.csv",
        help="Device GPU metadata CSV.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=SCRIPT_DIR / "charts",
        help="Directory in which charts are written.",
    )
    parser.add_argument(
        "--format",
        choices=("both", "pdf", "png", "svg"),
        default="both",
        help="Output image format; 'both' writes PNG and PDF.",
    )
    parser.add_argument("--dpi", type=int, default=180)
    parser.add_argument(
        "--drop-ratio",
        type=float,
        default=0.90,
        help="External/image FPS ratio considered a meaningful drop.",
    )
    return parser.parse_args()


def load_benchmarks(path: Path) -> dict[str, list[BenchmarkPoint]]:
    devices: dict[str, list[BenchmarkPoint]] = defaultdict(list)
    with path.open(newline="", encoding="utf-8") as file:
        for row in csv.DictReader(file):
            devices[row["device"]].append(
                BenchmarkPoint(
                    rect_count=int(row["rect_count"]),
                    image_fps=float(row["image_paint_fps"]),
                    external_fps=float(row["external_texture_fps"]),
                    image_cpu_ms=float(row["image_paint_cpu_ms"]),
                    external_cpu_ms=float(row["external_texture_cpu_ms"]),
                )
            )
    for points in devices.values():
        points.sort(key=lambda point: point.rect_count)
    return dict(devices)


def load_gpu_scores(path: Path) -> dict[str, float]:
    scores: dict[str, float] = {}
    with path.open(newline="", encoding="utf-8") as file:
        for row in csv.DictReader(file):
            value = row["geekbench_gpu_score"].strip()
            if value != "-":
                scores[row["device"]] = float(value)
    return scores


def device_legend_label(device: str, gpu_scores: dict[str, float]) -> str:
    score = gpu_scores.get(device)
    formatted_score = "n/a" if score is None else f"{score:,.0f}"
    return f"{device} (GB6: {formatted_score})"


def configure_style() -> None:
    plt.style.use("seaborn-v0_8-whitegrid")
    plt.rcParams.update(
        {
            "figure.dpi": 120,
            "font.size": 13,
            "axes.titlesize": 19,
            "axes.titleweight": "bold",
            "axes.labelsize": 15,
            "axes.spines.top": False,
            "axes.spines.right": False,
            "xtick.labelsize": 11,
            "ytick.labelsize": 12,
            "legend.fontsize": 12,
            "legend.title_fontsize": 13,
            "legend.frameon": False,
        }
    )


def save_figure(
    figure: plt.Figure, output_dir: Path, name: str, file_format: str, dpi: int
) -> Path:
    output = output_dir / f"{name}.{file_format}"
    figure.savefig(output, dpi=dpi, bbox_inches="tight")
    return output


def plot_fps_by_device(
    devices: dict[str, list[BenchmarkPoint]],
) -> plt.Figure:
    column_count = 3
    row_count = math.ceil(len(devices) / column_count)
    figure, axes = plt.subplots(
        row_count,
        column_count,
        figsize=(15, 3.6 * row_count),
        squeeze=False,
        sharex=False,
    )

    for axis, (device, points) in zip(axes.flat, devices.items(), strict=False):
        rects = [point.rect_count for point in points]
        axis.plot(
            rects,
            [point.image_fps for point in points],
            marker="o",
            markersize=3,
            label="Image paint",
        )
        axis.plot(
            rects,
            [point.external_fps for point in points],
            marker="o",
            markersize=3,
            label="External texture",
        )
        axis.set(title=device, xlabel="Rectangles", ylabel="FPS", xscale="log")
        axis.set_ylim(bottom=0)
        axis.legend()

    for axis in list(axes.flat)[len(devices) :]:
        axis.remove()

    figure.suptitle("Image paint vs. external-texture FPS", fontsize=16)
    figure.tight_layout()
    return figure


def plot_paired_fps(
    devices: dict[str, list[BenchmarkPoint]],
    gpu_scores: dict[str, float],
) -> plt.Figure:
    max_rect_count = 6000
    chart_devices = {
        device: [point for point in points if point.rect_count <= max_rect_count]
        for device, points in devices.items()
    }
    figure, axis = plt.subplots(figsize=(16, 9))
    colors = plt.colormaps["tab10"]
    device_handles: list[Line2D] = []
    rect_counts = sorted(
        {point.rect_count for points in chart_devices.values() for point in points}
    )

    for index, (device, points) in enumerate(chart_devices.items()):
        color = colors(index % colors.N)
        rects = [point.rect_count for point in points]
        image_fps = [point.image_fps for point in points]
        external_fps = [point.external_fps for point in points]

        axis.plot(
            rects,
            image_fps,
            color=color,
            linestyle="-",
            linewidth=1.8,
            marker="^",
            markersize=5.5,
            alpha=0.6,
            zorder=1,
        )
        axis.plot(
            rects,
            external_fps,
            color=color,
            linestyle="-",
            linewidth=2.3,
            marker="o",
            markersize=4.5,
            alpha=0.6,
            zorder=2,
        )
        axis.fill_between(
            rects,
            image_fps,
            external_fps,
            color=color,
            alpha=0.14,
            linewidth=0,
        )
        device_handles.append(
            Line2D(
                [0],
                [0],
                color=color,
                linewidth=3,
                label=device_legend_label(device, gpu_scores),
            )
        )

    method_handles = [
        Line2D(
            [0],
            [0],
            color="0.25",
            linestyle="-",
            linewidth=1.8,
            marker="^",
            markersize=7,
            alpha=0.6,
            label="Image paint (▲)",
        ),
        Line2D(
            [0],
            [0],
            color="0.25",
            linestyle="-",
            linewidth=2.3,
            marker="o",
            markersize=6,
            alpha=0.6,
            label="External texture (●)",
        ),
    ]
    figure.legend(
        handles=device_handles,
        title="Device color",
        bbox_to_anchor=(0.78, 0.91),
        loc="upper left",
    )
    figure.legend(
        handles=method_handles,
        title="Rendering method",
        bbox_to_anchor=(0.78, 0.18),
        loc="lower left",
    )
    axis.set(
        title="Image paint vs. external-texture FPS by device",
        xlabel="Rectangles",
        ylabel="FPS",
    )
    labeled_rect_counts = [1, 300, 600, *range(1000, max(rect_counts) + 1, 1000)]
    axis.set_xticks(labeled_rect_counts)
    tick_labels = axis.set_xticklabels(
        [f"{rect_count:,}" for rect_count in labeled_rect_counts],
        rotation=45,
        ha="right",
    )
    tick_labels[0].set(rotation=0, horizontalalignment="left")
    axis.set_xlim(0, max_rect_count)
    axis.set_ylim(bottom=0)
    figure.subplots_adjust(right=0.74, bottom=0.15)
    return figure


def plot_normalized_fps(
    devices: dict[str, list[BenchmarkPoint]], drop_ratio: float
) -> plt.Figure:
    figure, axis = plt.subplots(figsize=(11, 7))
    for device, points in devices.items():
        axis.plot(
            [point.rect_count for point in points],
            [point.external_fps / point.image_fps for point in points],
            marker="o",
            markersize=3,
            label=device,
        )

    axis.axhline(
        drop_ratio,
        color="black",
        linestyle="--",
        linewidth=1,
        label=f"{drop_ratio:.0%} threshold",
    )
    axis.set(
        title="External-texture FPS relative to image paint",
        xlabel="Rectangles",
        ylabel="External FPS / image FPS",
        xscale="log",
    )
    axis.legend(ncols=2, fontsize="small")
    figure.tight_layout()
    return figure


def plot_cpu_ms(
    devices: dict[str, list[BenchmarkPoint]],
    gpu_scores: dict[str, float],
) -> plt.Figure:
    max_rect_count = 6000
    chart_devices = {
        device: [point for point in points if point.rect_count <= max_rect_count]
        for device, points in devices.items()
    }
    figure, axis = plt.subplots(figsize=(16, 9))
    colors = plt.colormaps["tab10"]
    device_handles: list[Line2D] = []

    for index, (device, points) in enumerate(chart_devices.items()):
        color = colors(index % colors.N)
        rects = [point.rect_count for point in points]
        image_cpu_ms = [point.image_cpu_ms for point in points]
        external_cpu_ms = [point.external_cpu_ms for point in points]
        axis.plot(
            rects,
            image_cpu_ms,
            color=color,
            linewidth=1.8,
            marker="^",
            markersize=5.5,
            alpha=0.6,
        )
        axis.plot(
            rects,
            external_cpu_ms,
            color=color,
            linewidth=2.3,
            marker="o",
            markersize=4.5,
            alpha=0.6,
        )
        axis.fill_between(
            rects,
            image_cpu_ms,
            external_cpu_ms,
            color=color,
            alpha=0.14,
            linewidth=0,
        )
        device_handles.append(
            Line2D(
                [0],
                [0],
                color=color,
                linewidth=3,
                label=device_legend_label(device, gpu_scores),
            )
        )

    method_handles = [
        Line2D(
            [0],
            [0],
            color="0.25",
            linewidth=1.8,
            marker="^",
            markersize=7,
            alpha=0.6,
            label="Image paint (▲)",
        ),
        Line2D(
            [0],
            [0],
            color="0.25",
            linewidth=2.3,
            marker="o",
            markersize=6,
            alpha=0.6,
            label="External texture (●)",
        ),
    ]
    figure.legend(
        handles=device_handles,
        title="Device color",
        bbox_to_anchor=(0.78, 0.91),
        loc="upper left",
    )
    figure.legend(
        handles=method_handles,
        title="Rendering method",
        bbox_to_anchor=(0.78, 0.18),
        loc="lower left",
    )
    axis.set(
        title="Image paint vs. external-texture CPU time",
        xlabel="Rectangles",
        ylabel="CPU time per frame (ms)",
    )
    labeled_rect_counts = [1, 300, 600, *range(1000, max_rect_count + 1, 1000)]
    axis.set_xticks(labeled_rect_counts)
    tick_labels = axis.set_xticklabels(
        [f"{rect_count:,}" for rect_count in labeled_rect_counts],
        rotation=45,
        ha="right",
    )
    tick_labels[0].set(rotation=0, horizontalalignment="left")
    axis.set_xlim(0, max_rect_count)
    axis.set_ylim(bottom=0)
    figure.subplots_adjust(right=0.74, bottom=0.15)
    return figure


def plot_normalized_cpu_overhead(
    devices: dict[str, list[BenchmarkPoint]],
    gpu_scores: dict[str, float],
) -> plt.Figure:
    max_rect_count = 6000
    chart_devices = {
        device: [point for point in points if point.rect_count <= max_rect_count]
        for device, points in devices.items()
    }
    figure, axis = plt.subplots(figsize=(16, 9))
    colors = plt.colormaps["tab10"]
    device_handles: list[Line2D] = []

    for index, (device, points) in enumerate(chart_devices.items()):
        color = colors(index % colors.N)
        axis.plot(
            [point.rect_count for point in points],
            [point.external_cpu_ms / point.image_cpu_ms for point in points],
            color=color,
            linewidth=2.3,
            marker="o",
            markersize=4.5,
            alpha=0.6,
        )
        device_handles.append(
            Line2D(
                [0],
                [0],
                color=color,
                linewidth=3,
                label=device_legend_label(device, gpu_scores),
            )
        )

    axis.axhline(1, color="0.25", linestyle="--", linewidth=1.5, alpha=0.8)
    figure.legend(
        handles=device_handles,
        title="Device color",
        bbox_to_anchor=(0.78, 0.91),
        loc="upper left",
    )
    figure.legend(
        handles=[
            Line2D(
                [0],
                [0],
                color="0.25",
                linestyle="--",
                linewidth=1.5,
                label="1× = equal CPU time",
            )
        ],
        title="Reference",
        bbox_to_anchor=(0.78, 0.18),
        loc="lower left",
    )
    axis.set(
        title="External-texture CPU overhead normalized to image paint",
        xlabel="Rectangles",
        ylabel="External CPU time / image CPU time (×)",
    )
    labeled_rect_counts = [1, 300, 600, *range(1000, max_rect_count + 1, 1000)]
    axis.set_xticks(labeled_rect_counts)
    tick_labels = axis.set_xticklabels(
        [f"{rect_count:,}" for rect_count in labeled_rect_counts],
        rotation=45,
        ha="right",
    )
    tick_labels[0].set(rotation=0, horizontalalignment="left")
    axis.set_xlim(0, max_rect_count)
    axis.set_ylim(bottom=0)
    figure.subplots_adjust(right=0.74, bottom=0.15)
    return figure


def first_drop(
    points: list[BenchmarkPoint], drop_ratio: float
) -> int | None:
    return next(
        (
            point.rect_count
            for point in points
            if point.external_fps / point.image_fps < drop_ratio
        ),
        None,
    )


def plot_gpu_score_correlation(
    devices: dict[str, list[BenchmarkPoint]],
    gpu_scores: dict[str, float],
    drop_ratio: float,
) -> plt.Figure:
    figure, axis = plt.subplots(figsize=(10, 7))
    for device, points in devices.items():
        score = gpu_scores.get(device)
        threshold = first_drop(points, drop_ratio)
        if score is None or threshold is None:
            continue
        axis.scatter(score, threshold, s=65)
        axis.annotate(
            device,
            (score, threshold),
            xytext=(6, 5),
            textcoords="offset points",
            fontsize="small",
        )

    axis.set(
        title=f"GPU score vs. first external-FPS drop below {drop_ratio:.0%}",
        xlabel="Geekbench GPU score",
        ylabel="Rectangle count at first drop",
    )
    figure.tight_layout()
    return figure


def main() -> None:
    args = parse_args()
    configure_style()
    devices = load_benchmarks(args.benchmarks)
    gpu_scores = load_gpu_scores(args.gpus)
    args.output_dir.mkdir(parents=True, exist_ok=True)

    figures = {
        "paired_fps": plot_paired_fps(devices, gpu_scores),
        "cpu_ms": plot_cpu_ms(devices, gpu_scores),
        "cpu_overhead_normalized": plot_normalized_cpu_overhead(
            devices, gpu_scores
        ),
    }
    file_formats = ("png", "pdf") if args.format == "both" else (args.format,)
    for name, figure in figures.items():
        for file_format in file_formats:
            output = save_figure(
                figure, args.output_dir, name, file_format, args.dpi
            )
            print(output)
        plt.close(figure)


if __name__ == "__main__":
    main()
