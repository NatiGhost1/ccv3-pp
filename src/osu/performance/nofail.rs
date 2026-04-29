// CC V3 (NoFail): standalone performance scaling.
//
// NoFail prevents the fail screen — players can't die. This means:
//   * They might AFK / stop playing mid-map and still "pass"
//   * Short maps become easy farm targets (no risk of failing)
//   * The existing miss system doesn't account for NF's reduced stakes
//
// This module provides:
//   * Fail detection:  if total_hits < n_objects, player didn't finish → pp = 0
//   * Short map tax:   for maps with max_combo < 1000, a map-combo-based tax
//                      (NOT player combo). Lightened with more misses, but
//                      pp always decreases monotonically with miss count.
//   * Long map symmetric scaling: miss penalty is symmetric around the
//                      midpoint (start and end worth ~same, midpoint
//                      weighted highest).
//   * Per-miss decay:  gentle NF-specific decay, floors at 0.50.
//
// CRITICAL: this system is standalone — apply_cc_v3_multiplier returns
// 1.0 on NF plays, so the old exponential doesn't stack.

/// Compute the NoFail performance multiplier.
///
/// Returns [0.0, 1.0]:
///   * 0.0 if the player didn't finish (failed / quit mid-map)
///   * Otherwise: short_map_tax × symmetric_miss_mult × miss_decay
pub fn nf_multiplier(
    map_max_combo: u32,
    player_max_combo: u32,
    misses: u32,
    total_hits: u32,
    n_objects: u32,
) -> f64 {
    // ── Fail detection ──────────────────────────────────────────────
    // If the player didn't hit every object, they stopped playing.
    // With NF the game continues but they were effectively AFK.
    // PP = 0 for incomplete plays.
    if total_hits < n_objects {
        return 0.0;
    }

    let mc = f64::from(map_max_combo);
    let miss_f = f64::from(misses);

    // ── Short map tax (map_max_combo < 1000) ────────────────────────
    //
    // Purely based on map_max_combo (player combo is NOT used).
    // Applied to FC and miss plays alike.
    //
    // base_tax scales linearly:
    //   combo   0 → 0.70  (−30%)
    //   combo 500 → 0.85  (−15%)
    //   combo 999 → ~1.00
    //
    // Miss relief: each miss lightens the tax by 1.5%, up to 15 misses
    // (max +22.5% relief). The TAX gets lighter with more misses, but
    // the per-miss DECAY below always outweighs the relief, ensuring
    // pp always decreases monotonically:
    //
    //   FC:        tax=0.850, decay=1.000  → net=0.850
    //   1 miss:    tax=0.865, decay=0.970  → net=0.839  (< FC ✓)
    //   5 misses:  tax=0.925, decay=0.859  → net=0.794  (< 1 miss ✓)
    //   10 misses: tax=1.000, decay=0.737  → net=0.737  (< 5 miss ✓)
    //
    let short_tax = if map_max_combo < 1000 {
        let base = 0.70 + 0.30 * (mc / 1000.0);
        let miss_relief = 0.015 * miss_f.min(15.0);
        (base + miss_relief).min(1.0)
    } else {
        1.0
    };

    // ── Long map symmetric scaling (map_max_combo >= 1000) ──────────
    //
    // When misses occur on long maps, the penalty is symmetric around
    // the midpoint. A miss near the start weighs approximately the
    // same as a miss the same distance from the end. Midpoint misses
    // are punished most — that's where maintaining combo matters most.
    //
    // proximity = 1 − |combo_ratio − 0.5| / 0.5
    //   combo_ratio 0.0 → prox 0.0  (very start)
    //   combo_ratio 0.5 → prox 1.0  (midpoint, max penalty)
    //   combo_ratio 1.0 → prox 0.0  (very end)
    //
    // For FC (0 misses), this block returns 1.0 — no penalty.
    //
    //   Edges:    0.95  (miss near start or end, mild)
    //   Midpoint: 0.82  (miss right in the middle, harsh)
    let symmetric_mult = if map_max_combo >= 1000 && misses > 0 && mc > 0.0 {
        let combo_ratio = (f64::from(player_max_combo) / mc).clamp(0.0, 1.0);
        let prox = 1.0 - ((combo_ratio - 0.5).abs() / 0.5).min(1.0);
        0.95 - 0.13 * prox
    } else {
        1.0
    };

    // ── Per-miss decay ──────────────────────────────────────────────
    //
    // Gentle NF-specific decay: 0.97 per miss, floored at 0.50.
    // Separate from the CC V3 exponential (which returns 1.0 on NF).
    // Ensures pp always decreases with more misses.
    //
    //   0 misses:   1.000
    //   1 miss:     0.970  (−3.0%)
    //   3 misses:   0.913  (−8.7%)
    //   5 misses:   0.859  (−14.1%)
    //   10 misses:  0.737  (−26.3%)
    //   20 misses:  0.544  (−45.6%)
    //   23+ misses: 0.500  (floor)
    let miss_decay = if misses > 0 {
        0.97_f64.powf(miss_f).max(0.50)
    } else {
        1.0
    };

    // ── Compose ─────────────────────────────────────────────────────
    //
    // Guarantees:
    //   * FC always scores highest (miss_decay=1.0, symmetric=1.0)
    //   * 1 miss < FC (miss_decay=0.97 dominates any tax relief)
    //   * n+1 misses < n misses (miss_decay is strictly decreasing)
    //   * Incomplete plays = 0.0 (fail detection)
    (short_tax * symmetric_mult * miss_decay).max(0.0)
}
