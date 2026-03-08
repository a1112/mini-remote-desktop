#!/usr/bin/env python3
"""Resilience tests for benchmark utilities."""

import benchmark


def test_safe_latency_stats_returns_none_for_empty_series():
    assert benchmark.safe_latency_stats([]) is None


def test_safe_latency_stats_computes_core_metrics():
    stats = benchmark.safe_latency_stats([1.0, 2.0, 3.0, 4.0, 5.0])
    assert stats is not None
    assert stats["avg"] == 3.0
    assert stats["max"] == 5.0
    assert stats["min"] == 1.0
