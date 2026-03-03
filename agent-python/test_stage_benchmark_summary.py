#!/usr/bin/env python3
"""Tests for stage benchmark summary helpers."""

import stage_benchmark


def test_summarize_stage_metrics_handles_empty():
    summary = stage_benchmark.summarize_stage_metrics([])
    assert summary["frames"] == 0
    assert summary["capture_ms"] is None


def test_summarize_stage_metrics_computes_averages():
    rows = [
        {"capture_ms": 1.0, "encode_ms": 2.0, "send_ms": 3.0, "total_ms": 6.0, "encoded_size": 1000},
        {"capture_ms": 2.0, "encode_ms": 4.0, "send_ms": 6.0, "total_ms": 12.0, "encoded_size": 3000},
    ]
    summary = stage_benchmark.summarize_stage_metrics(rows)
    assert summary["frames"] == 2
    assert summary["capture_ms"]["avg"] == 1.5
    assert summary["encode_ms"]["avg"] == 3.0
    assert summary["send_ms"]["avg"] == 4.5
