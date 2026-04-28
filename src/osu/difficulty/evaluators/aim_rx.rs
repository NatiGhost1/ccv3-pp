// CC V3: Relax-specific aim evaluator for rosu-pp.
//
// Based on rosu-pp's AimEvaluator (evaluators/aim.rs) with:
//   * Uplifted angle multipliers for RX-relevant patterns
//   * Windowed angle variance (replaces pairwise repetition)
//   * Slow-slider velocity taper
//   * Cross-screen constant-distance nerf
//   * Distance-gated extreme flow aim nerf
//   * High-BPM repetition buff preserved
//
// Uses rosu's formula structure (smoothstep/smootherstep, adjusted_delta_time,
// wiggle_bonus, small_circle_bonus, DIAMETER-based gating).

use std::f64::consts::PI;

use crate::{
    any::difficulty::object::IDifficultyObject,
    osu::difficulty::object::OsuDifficultyObject,
    util::{
        difficulty::{milliseconds_to_bpm, reverse_lerp, smootherstep, smoothstep},
        float_ext::FloatExt,
    },
};

pub struct AimRxEvaluator;

// ─── Windowed angle statistics ──────────────────────────────────────

const ANGLE_WINDOW: usize = 8;

fn windowed_angle_stats<'a>(
    curr: &'a OsuDifficultyObject<'a>,
    diff_objects: &'a [OsuDifficultyObject<'a>],
    window: usize,
) -> (f64, f64, usize) {
    let mut angles: Vec<f64> = Vec::with_capacity(window + 1);

    if let Some(a) = curr.angle {
        angles.push(a);
    }
    for back in 0..window {
        if let Some(prev) = curr.previous(back, diff_objects) {
            if let Some(a) = prev.angle {
                angles.push(a);
            }
        } else {
            break;
        }
    }

    let n = angles.len();
    if n < 3 {
        return (0.0, 0.0, n);
    }

    let mean: f64 = angles.iter().sum::<f64>() / n as f64;
    let variance: f64 = angles.iter().map(|a| (a - mean).powi(2)).sum::<f64>() / n as f64;
    (mean, variance.sqrt(), n)
}

/// Average lazy_jump_dist over a lookback window.
fn windowed_dist_mean<'a>(
    curr: &'a OsuDifficultyObject<'a>,
    diff_objects: &'a [OsuDifficultyObject<'a>],
    window: usize,
) -> (f64, usize) {
    let mut dists: Vec<f64> = Vec::with_capacity(window + 1);
    dists.push(curr.lazy_jump_dist);

    for back in 0..window {
        if let Some(prev) = curr.previous(back, diff_objects) {
            dists.push(prev.lazy_jump_dist);
        } else {
            break;
        }
    }

    let n = dists.len();
    if n == 0 {
        return (0.0, 0);
    }
    (dists.iter().sum::<f64>() / n as f64, n)
}

impl AimRxEvaluator {
    // Uplifted from rosu vanilla (1.5 / 2.55 / 1.35 / 0.75 / 1.02)
    // Same % uplift as the akat-based version relative to its vanilla.
    const WIDE_ANGLE_MULTIPLIER: f64 = 1.62;    // +8% vs rosu 1.5
    const ACUTE_ANGLE_MULTIPLIER: f64 = 2.75;   // +8% vs rosu 2.55
    const SLIDER_MULTIPLIER: f64 = 1.20;        // -11% (sliders farmed on RX)
    const VELOCITY_CHANGE_MULTIPLIER: f64 = 0.84; // +12% (vel change = real aim)
    const WIGGLE_MULTIPLIER: f64 = 1.02;        // same as vanilla

    // Slow slider taper
    const SLOW_SLIDER_VEL_FLOOR: f64 = 0.55;

    // Cross-screen constant-distance nerf
    const CONSTANT_DIST_RATIO: f64 = 0.18;
    const EDGE_TO_EDGE_THRESHOLD: f64 = 400.0;
    const CONSTANT_DIST_BPM_STRAIN_TIME: f64 = 85.7;

    // Flow aim nerf (distance-gated)
    const FLOW_MEAN_ANGLE_THRESHOLD: f64 = 2.0;
    const FLOW_STDDEV_THRESHOLD: f64 = 0.3;
    const FLOW_MAX_NERF: f64 = 0.50;
    const FLOW_BPM_STRAIN_TIME: f64 = 36.58;
    const FLOW_DIST_FULL_NERF: f64 = 50.0;
    const FLOW_DIST_EXEMPT: f64 = 120.0;

    pub fn evaluate_diff_of<'a>(
        curr: &'a OsuDifficultyObject<'a>,
        diff_objects: &'a [OsuDifficultyObject<'a>],
        with_slider_travel_dist: bool,
    ) -> f64 {
        let osu_curr_obj = curr;

        let Some((osu_last_last_obj, osu_last_obj)) = curr
            .previous(1, diff_objects)
            .zip(curr.previous(0, diff_objects))
            .filter(|(_, last)| !(curr.base.is_spinner() || last.base.is_spinner()))
        else {
            return 0.0;
        };

        const RADIUS: i32 = OsuDifficultyObject::NORMALIZED_RADIUS;
        const DIAMETER: i32 = OsuDifficultyObject::NORMALIZED_DIAMETER;

        // ── Velocities (using adjusted_delta_time like rosu) ────────
        let mut curr_vel = osu_curr_obj.lazy_jump_dist / osu_curr_obj.adjusted_delta_time;

        if osu_last_obj.base.is_slider() && with_slider_travel_dist {
            let travel_vel = osu_last_obj.travel_dist / osu_last_obj.travel_time;
            let movement_vel = osu_curr_obj.min_jump_dist / osu_curr_obj.min_jump_time;
            curr_vel = curr_vel.max(movement_vel + travel_vel);
        }

        let mut prev_vel = osu_last_obj.lazy_jump_dist / osu_last_obj.adjusted_delta_time;

        if osu_last_last_obj.base.is_slider() && with_slider_travel_dist {
            let travel_vel = osu_last_last_obj.travel_dist / osu_last_last_obj.travel_time;
            let movement_vel = osu_last_obj.min_jump_dist / osu_last_obj.min_jump_time;
            prev_vel = prev_vel.max(movement_vel + travel_vel);
        }

        let mut wide_angle_bonus = 0.0;
        let mut acute_angle_bonus = 0.0;
        let mut slider_bonus = 0.0;
        let mut vel_change_bonus = 0.0;
        let mut wiggle_bonus = 0.0;

        let mut aim_strain = curr_vel;

        // ── Angle bonuses (rosu-style smoothstep) ───────────────────
        if let Some((curr_angle, last_angle)) = osu_curr_obj.angle.zip(osu_last_obj.angle) {
            let angle_bonus = curr_vel.min(prev_vel);

            // Rhythm consistency check (same as vanilla)
            if osu_curr_obj
                .adjusted_delta_time
                .max(osu_last_obj.adjusted_delta_time)
                < 1.25
                    * osu_curr_obj
                        .adjusted_delta_time
                        .min(osu_last_obj.adjusted_delta_time)
            {
                acute_angle_bonus = Self::calc_acute_angle_bonus(curr_angle);

                // Vanilla-style pairwise acute repetition penalty
                acute_angle_bonus *= 0.08
                    + 0.92
                        * (1.0
                            - f64::min(
                                acute_angle_bonus,
                                f64::powf(Self::calc_acute_angle_bonus(last_angle), 3.0),
                            ));

                // BPM + distance gating (rosu smootherstep)
                acute_angle_bonus *= angle_bonus
                    * smootherstep(
                        milliseconds_to_bpm(osu_curr_obj.adjusted_delta_time, Some(2)),
                        300.0,
                        400.0,
                    )
                    * smootherstep(
                        osu_curr_obj.lazy_jump_dist,
                        f64::from(DIAMETER),
                        f64::from(DIAMETER * 2),
                    );
            }

            wide_angle_bonus = Self::calc_wide_angle_bonus(curr_angle);

            // ── Windowed variance repetition (CC V3 RX addition) ────
            // Replaces vanilla pairwise wide penalty with a proper
            // window-based variance measure.
            let eff_bpm = 30_000.0 / osu_curr_obj.adjusted_delta_time;
            let high_bpm_t = ((eff_bpm - 410.0) / 90.0).clamp(0.0, 1.0);

            let (_win_mean, win_stddev, win_n) =
                windowed_angle_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);

            let variance_factor = if win_n >= 3 {
                (win_stddev / 1.2).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let rep_strength = 1.0 - variance_factor;

            let wide_penalty = rep_strength * (1.0 - high_bpm_t);
            let wide_rep_buff = high_bpm_t * 0.15;
            wide_angle_bonus *=
                angle_bonus * smootherstep(osu_curr_obj.lazy_jump_dist, 0.0, f64::from(DIAMETER))
                * (1.0 - wide_penalty + wide_rep_buff).max(0.0);

            // Wiggle bonus (preserved from rosu vanilla)
            wiggle_bonus = angle_bonus
                * smootherstep(
                    osu_curr_obj.lazy_jump_dist,
                    f64::from(RADIUS),
                    f64::from(DIAMETER),
                )
                * f64::powf(
                    reverse_lerp(
                        osu_curr_obj.lazy_jump_dist,
                        f64::from(DIAMETER * 3),
                        f64::from(DIAMETER),
                    ),
                    1.8,
                )
                * smootherstep(curr_angle, f64::to_radians(110.0), f64::to_radians(60.0))
                * smootherstep(
                    osu_last_obj.lazy_jump_dist,
                    f64::from(RADIUS),
                    f64::from(DIAMETER),
                )
                * f64::powf(
                    reverse_lerp(
                        osu_last_obj.lazy_jump_dist,
                        f64::from(DIAMETER * 3),
                        f64::from(DIAMETER),
                    ),
                    1.8,
                )
                * smootherstep(last_angle, f64::to_radians(110.0), f64::to_radians(60.0));

            // Stacked object penalty (from rosu vanilla)
            if let Some(osu_last_2_obj) = curr.previous(2, diff_objects) {
                let distance =
                    (osu_last_2_obj.base.stacked_pos() - osu_last_obj.base.stacked_pos()).length();
                if distance < 1.0 {
                    wide_angle_bonus *= 1.0 - 0.35 * f64::from(1.0 - distance);
                }
            }
        }

        // ── Velocity change bonus (rosu smoothstep) ─────────────────
        if prev_vel.max(curr_vel).not_eq(0.0) {
            prev_vel = (osu_last_obj.lazy_jump_dist + osu_last_last_obj.travel_dist)
                / osu_last_obj.adjusted_delta_time;
            curr_vel = (osu_curr_obj.lazy_jump_dist + osu_last_obj.travel_dist)
                / osu_curr_obj.adjusted_delta_time;

            let dist_ratio = smoothstep(
                (prev_vel - curr_vel).abs() / prev_vel.max(curr_vel),
                0.0,
                1.0,
            );

            let overlap_vel_buff = (f64::from(DIAMETER) * 1.25
                / osu_curr_obj
                    .adjusted_delta_time
                    .min(osu_last_obj.adjusted_delta_time))
            .min((prev_vel - curr_vel).abs());

            vel_change_bonus = overlap_vel_buff * dist_ratio;

            let bonus_base = osu_curr_obj
                .adjusted_delta_time
                .min(osu_last_obj.adjusted_delta_time)
                / osu_curr_obj
                    .adjusted_delta_time
                    .max(osu_last_obj.adjusted_delta_time);
            vel_change_bonus *= bonus_base.powf(2.0);
        }

        // ── Slider bonus with slow-slider taper (CC V3 RX) ─────────
        if osu_last_obj.base.is_slider() {
            let travel_vel = osu_last_obj.travel_dist / osu_last_obj.travel_time;
            slider_bonus = travel_vel;

            if travel_vel < Self::SLOW_SLIDER_VEL_FLOOR {
                let ratio = (travel_vel / Self::SLOW_SLIDER_VEL_FLOOR).clamp(0.0, 1.0);
                slider_bonus *= 0.55 + 0.45 * ratio;
            }
        }

        // ── Combine (rosu order) ────────────────────────────────────
        aim_strain += wiggle_bonus * Self::WIGGLE_MULTIPLIER;
        aim_strain += vel_change_bonus * Self::VELOCITY_CHANGE_MULTIPLIER;

        aim_strain += (acute_angle_bonus * Self::ACUTE_ANGLE_MULTIPLIER)
            .max(wide_angle_bonus * Self::WIDE_ANGLE_MULTIPLIER);

        aim_strain *= osu_curr_obj.small_circle_bonus;

        if with_slider_travel_dist {
            aim_strain += slider_bonus * Self::SLIDER_MULTIPLIER;
        }

        // ── Cross-screen constant-distance nerf (CC V3 RX) ─────────
        if osu_curr_obj.adjusted_delta_time >= Self::CONSTANT_DIST_BPM_STRAIN_TIME {
            let curr_d = osu_curr_obj.lazy_jump_dist;
            let prev_d = osu_last_obj.lazy_jump_dist;
            let max_d = curr_d.max(prev_d);
            let min_d = curr_d.min(prev_d);

            if max_d > 80.0 {
                let change_ratio = if max_d > 0.0 {
                    (max_d - min_d) / max_d
                } else {
                    1.0
                };

                let is_edge_to_edge = max_d >= Self::EDGE_TO_EDGE_THRESHOLD;

                if !is_edge_to_edge && change_ratio < Self::CONSTANT_DIST_RATIO {
                    let ratio_factor = 1.0 - (change_ratio / Self::CONSTANT_DIST_RATIO);
                    let dist_factor = 1.0
                        - ((max_d - 80.0) / (Self::EDGE_TO_EDGE_THRESHOLD - 80.0))
                            .clamp(0.0, 1.0);
                    let severity = ratio_factor * dist_factor;
                    aim_strain *= 1.0 - 0.15 * severity;
                }
            }
        }

        // ── Extreme flow aim nerf (distance-gated, CC V3 RX) ────────
        if osu_curr_obj.adjusted_delta_time >= Self::FLOW_BPM_STRAIN_TIME {
            let (flow_mean, flow_stddev, flow_n) =
                windowed_angle_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);

            if flow_n >= 4 {
                let mean_ok = flow_mean >= Self::FLOW_MEAN_ANGLE_THRESHOLD;
                let stddev_ok = flow_stddev <= Self::FLOW_STDDEV_THRESHOLD;

                if mean_ok && stddev_ok {
                    let stddev_severity =
                        (1.0 - (flow_stddev / Self::FLOW_STDDEV_THRESHOLD)).powi(2);
                    let mean_range = PI - Self::FLOW_MEAN_ANGLE_THRESHOLD;
                    let mean_severity = ((flow_mean - Self::FLOW_MEAN_ANGLE_THRESHOLD)
                        / mean_range)
                        .clamp(0.0, 1.0);
                    let angle_severity = stddev_severity * mean_severity;

                    let (avg_dist, dist_n) =
                        windowed_dist_mean(osu_curr_obj, diff_objects, ANGLE_WINDOW);

                    let dist_factor = if dist_n < 3 {
                        0.5
                    } else if avg_dist <= Self::FLOW_DIST_FULL_NERF {
                        1.0
                    } else if avg_dist >= Self::FLOW_DIST_EXEMPT {
                        0.0
                    } else {
                        1.0 - ((avg_dist - Self::FLOW_DIST_FULL_NERF)
                            / (Self::FLOW_DIST_EXEMPT - Self::FLOW_DIST_FULL_NERF))
                    };

                    let combined = angle_severity * dist_factor;
                    aim_strain *= 1.0 - Self::FLOW_MAX_NERF * combined;
                }
            }
        }

        aim_strain
    }

    const fn calc_wide_angle_bonus(angle: f64) -> f64 {
        smoothstep(angle, f64::to_radians(40.0), f64::to_radians(140.0))
    }

    const fn calc_acute_angle_bonus(angle: f64) -> f64 {
        smoothstep(angle, f64::to_radians(140.0), f64::to_radians(40.0))
    }
}