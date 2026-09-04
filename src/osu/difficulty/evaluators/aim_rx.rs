// CC V3: Relax-specific aim evaluator.
//
// Purpose:
//   * N/X pattern detection — detects alternating zigzag and crossover patterns
//     that are trivial on RX via angle pair analysis and distance consistency.
//   * Aim slop detection — identifies constant velocity + constant distance +
//     low angle variance windows (mechanical patterns) for progressive nerfing.
//   * Tech pattern boost — rewards high angle variance + high velocity change
//     ratio as genuine technical aim.
//   * Neutral flow protection — prevents accidental tech buffs on common flow
//     patterns that stay inside typical distance bands (112-90, 90-70, etc.).
//   * Delayed tech buff — avoids immediate tech application right after farm
//     sections to prevent disproportionate strain from isolated anomalies.
//   * Tech boost overall cap — limits maximum positive adjustment.
//   * Aim calibration — Calibrates aim values to reflect true skill,
//     compensating for ccv3-pp evaluation bias.
//
// Rework Additions:
//   * Pseudo-stacks — No pp awarded for objects too close together (under 10px).
//   * Stack-to-Dense Transitions — Penalizes moving from pseudo-stacks to dense spaced objects.
//   * Precision Scaling — Nerfs have reduced impact on smaller circle sizes.
//   * Cross-Screen Adjustments — Only triggers if accompanied by N/X or slop patterns.
//   * Slider Breakability — Short slider travel distances grant no bonus.
//   * Hybrid Section Buff — Buffs patterns with high distance variance (flow to jumps) 
//     that do not trigger neutral flow.
//
// Uses rosu's formula base (smoothstep/smootherstep, adjusted_delta_time,
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

// ─── Windowed statistics helpers ────────────────────────────────────

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

fn windowed_dist_stats<'a>(
    curr: &'a OsuDifficultyObject<'a>,
    diff_objects: &'a [OsuDifficultyObject<'a>],
    window: usize,
) -> (f64, f64, usize) {
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
    if n < 2 {
        return (0.0, 0.0, n);
    }
    let mean = dists.iter().sum::<f64>() / n as f64;
    let var = dists.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n as f64;
    (mean, var.sqrt(), n)
}

fn windowed_vel_stats<'a>(
    curr: &'a OsuDifficultyObject<'a>,
    diff_objects: &'a [OsuDifficultyObject<'a>],
    window: usize,
) -> (f64, f64, usize) {
    let mut vels: Vec<f64> = Vec::with_capacity(window + 1);
    if curr.adjusted_delta_time > 0.0 {
        vels.push(curr.lazy_jump_dist / curr.adjusted_delta_time);
    }

    for back in 0..window {
        if let Some(prev) = curr.previous(back, diff_objects) {
            if prev.adjusted_delta_time > 0.0 {
                vels.push(prev.lazy_jump_dist / prev.adjusted_delta_time);
            }
        } else {
            break;
        }
    }

    let n = vels.len();
    if n < 2 {
        return (0.0, 0.0, n);
    }
    let mean = vels.iter().sum::<f64>() / n as f64;
    let var = vels.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64;
    (mean, var.sqrt(), n)
}

/// Windowed stats over the gap (delta) TIMES of recent objects — used to spot
/// a "stream signature": a consistent, fast 1/4 rhythm. Returns
/// (mean_delta_ms, stddev_delta_ms, count).
fn windowed_delta_stats<'a>(
    curr: &'a OsuDifficultyObject<'a>,
    diff_objects: &'a [OsuDifficultyObject<'a>],
    window: usize,
) -> (f64, f64, usize) {
    let mut deltas: Vec<f64> = Vec::with_capacity(window + 1);
    if curr.adjusted_delta_time > 0.0 {
        deltas.push(curr.adjusted_delta_time);
    }

    for back in 0..window {
        if let Some(prev) = curr.previous(back, diff_objects) {
            if prev.adjusted_delta_time > 0.0 {
                deltas.push(prev.adjusted_delta_time);
            }
        } else {
            break;
        }
    }

    let n = deltas.len();
    if n < 2 {
        return (0.0, 0.0, n);
    }
    let mean = deltas.iter().sum::<f64>() / n as f64;
    let var = deltas.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n as f64;
    (mean, var.sqrt(), n)
}

/// Detect N/X alternating patterns: look at consecutive angle pairs
/// and check if they alternate between two values (±tolerance).
fn detect_nx_pattern<'a>(
    curr: &'a OsuDifficultyObject<'a>,
    diff_objects: &'a [OsuDifficultyObject<'a>],
    window: usize,
) -> f64 {
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

    if angles.len() < 4 {
        return 0.0;
    }

    let evens: Vec<f64> = angles.iter().step_by(2).copied().collect();
    let odds: Vec<f64> = angles.iter().skip(1).step_by(2).copied().collect();

    if evens.len() < 2 || odds.len() < 2 {
        return 0.0;
    }

    let even_mean = evens.iter().sum::<f64>() / evens.len() as f64;
    let odd_mean = odds.iter().sum::<f64>() / odds.len() as f64;

    let even_var = evens.iter().map(|a| (a - even_mean).powi(2)).sum::<f64>() / evens.len() as f64;
    let odd_var = odds.iter().map(|a| (a - odd_mean).powi(2)).sum::<f64>() / odds.len() as f64;

    let even_stddev = even_var.sqrt();
    let odd_stddev = odd_var.sqrt();

    let cluster_tight = even_stddev < 0.25 && odd_stddev < 0.25;
    let clusters_differ = (even_mean - odd_mean).abs() > 0.3;

    if cluster_tight && clusters_differ {
        let tightness = 1.0 - ((even_stddev + odd_stddev) / 0.50).clamp(0.0, 1.0);
        let separation = ((even_mean - odd_mean).abs() / PI).clamp(0.0, 1.0);
        tightness * separation
    } else {
        0.0
    }
}

impl AimRxEvaluator {
    // Recalibrated constants to produce skill-equivalent pp output.
    const WIDE_ANGLE_MULTIPLIER: f64 = 1.56;
    const ACUTE_ANGLE_MULTIPLIER: f64 = 2.66;
    const SLIDER_MULTIPLIER: f64 = 1.03;
    const VELOCITY_CHANGE_MULTIPLIER: f64 = 0.81;
    const WIGGLE_MULTIPLIER: f64 = 1.02;
    const AIM_CALIBRATION: f64 = 0.92;

    // Stack and flow spacing thresholds. Small jumps must not be mistaken for
    // flow, even when their rhythm is fast and regular.
    const PSEUDO_STACK_THRESHOLD: f64 = 10.0;
    const DENSE_SPACING_THRESHOLD: f64 = 55.0;

    // ── Stream-signature tech-buff gate (CC V3) ─────────────────────
    // A "stream signature" is a consistent, fast 1/4 RHYTHM — regardless of
    // spatial spacing. Spaced streams (similar patterns but spaced) have the
    // same even rhythm as a normal stream but larger jumps, which creates
    // velocity variety and can otherwise trip the tech buff. Under Relax that's
    // still just a stream, so the tech buff must NOT apply. Detection is purely
    // rhythmic: short mean gap (fast) + low gap-time variance (even rhythm).
    const STREAM_SIG_BPM_MIN: f64 = 180.0; // mean 1/4 BPM at/above which it's "fast"
    const STREAM_SIG_CV_MAX: f64 = 0.18; // gap-time coeff. of variation below which it's "even"
    const STREAM_SIG_MIN_NOTES: usize = 5; // need a real run, not 2-3 notes

    const SLOW_SLIDER_VEL_FLOOR: f64 = 0.55;

    const FOLLOW_POINT_DISTANCE: f64 = 112.0;
    const FARM_MAX_NERF: f64 = 0.35;
    const RELAX_REPEAT_NERF_RADIUS_START: f64 = 36.9;
    const RELAX_REPEAT_NERF_RADIUS_END: f64 = 24.4;
    const RELAX_REPEAT_NERF_EXPONENT: f64 = 9.0;
    const CONSTANT_DIST_RATIO: f64 = 0.18;
    const EDGE_TO_EDGE_THRESHOLD: f64 = 360.0;
    const CONSTANT_DIST_BPM_STRAIN_TIME: f64 = 85.7;

    const FLOW_MIN_EFF_BPM: f64 = 210.25;
    const FLOW_MEAN_ANGLE_THRESHOLD: f64 = 2.0;
    const FLOW_STDDEV_THRESHOLD: f64 = 0.3;
    const FLOW_MAX_NERF: f64 = 0.70;
    const FLOW_DIST_MIN: f64 = 55.0;
    const FLOW_DIST_FULL: f64 = 90.0;
    const FLOW_DIST_EXEMPT: f64 = 150.0;

    #[warn(dead_code)]
    const NX_MAX_NERF: f64 = 0.30;
    #[warn(dead_code)]
    const SLOP_MAX_NERF: f64 = 0.35;
    
    const TECH_MAX_BOOST: f64 = 0.08;
    const HYBRID_MAX_BOOST: f64 = 0.12;
    
    // Additional tuning constants
    const TECH_OVERALL_CAP: f64 = 1.08;

    const NEUTRAL_FLOW_DIST_RANGES: [(f64, f64); 5] = [
        (90.0, 112.0),
        (70.0, 90.0),
        (50.0, 70.0),
        (30.0, 50.0),
        (10.0, 30.0),
    ];

    /// True when recent objects form a stream signature: a consistent, fast
    /// 1/4 rhythm. Spacing is intentionally ignored, so spaced streams count.
    fn is_stream_pattern<'a>(
        curr: &'a OsuDifficultyObject<'a>,
        diff_objects: &'a [OsuDifficultyObject<'a>],
    ) -> bool {
        let (delta_mean, delta_stddev, n) =
            windowed_delta_stats(curr, diff_objects, ANGLE_WINDOW);
        if n < Self::STREAM_SIG_MIN_NOTES || delta_mean <= 0.0 {
            return false;
        }
        let eff_bpm = milliseconds_to_bpm(delta_mean, None);
        let cv = delta_stddev / delta_mean;
        eff_bpm >= Self::STREAM_SIG_BPM_MIN && cv <= Self::STREAM_SIG_CV_MAX
    }

    fn relax_repeat_nerf_radius_factor(circle_radius: f64) -> f64 {
        if circle_radius >= Self::RELAX_REPEAT_NERF_RADIUS_START {
            return 1.0;
        }

        let ratio = ((Self::RELAX_REPEAT_NERF_RADIUS_START - circle_radius)
            / (Self::RELAX_REPEAT_NERF_RADIUS_START - Self::RELAX_REPEAT_NERF_RADIUS_END))
            .clamp(0.0, 1.0);

        f64::exp(-Self::RELAX_REPEAT_NERF_EXPONENT * ratio)
    }

    fn is_neutral_flow_pattern<'a>(
        curr: &'a OsuDifficultyObject<'a>,
        diff_objects: &'a [OsuDifficultyObject<'a>],
    ) -> bool {
        let (dist_mean, dist_stddev, n) = windowed_dist_stats(curr, diff_objects, ANGLE_WINDOW);
        if n < 6 || dist_mean < 5.0 {
            return false;
        }

        let cv = dist_stddev / dist_mean;
        if cv > 0.25 {
            return false;
        }

        Self::NEUTRAL_FLOW_DIST_RANGES.iter().any(|&(low, high)| {
            dist_mean >= low && dist_mean <= high
        })
    }

    fn combined_flow_nerf<'a>(
        curr: &'a OsuDifficultyObject<'a>,
        diff_objects: &'a [OsuDifficultyObject<'a>],
    ) -> f64 {
        let transition = if curr.lazy_jump_dist > Self::PSEUDO_STACK_THRESHOLD
            && curr.lazy_jump_dist <= Self::DENSE_SPACING_THRESHOLD
            && curr
                .previous(0, diff_objects)
                .is_some_and(|prev| prev.lazy_jump_dist <= Self::PSEUDO_STACK_THRESHOLD)
        {
            0.35 * (curr.circle_radius / 36.0).clamp(0.2, 1.0)
        } else {
            0.0
        };

        let (angle_mean, angle_stddev, angle_n) =
            windowed_angle_stats(curr, diff_objects, ANGLE_WINDOW);
        let (dist_mean, dist_stddev, dist_n) =
            windowed_dist_stats(curr, diff_objects, ANGLE_WINDOW);

        if angle_n < 4 || dist_n < 4 || dist_mean < Self::FLOW_DIST_MIN {
            return Self::FLOW_MAX_NERF * transition;
        }

        let shape = ((angle_mean - Self::FLOW_MEAN_ANGLE_THRESHOLD)
            / (PI - Self::FLOW_MEAN_ANGLE_THRESHOLD))
            .clamp(0.0, 1.0)
            * (1.0 - (angle_stddev / Self::FLOW_STDDEV_THRESHOLD).clamp(0.0, 1.0)).powi(2);

        let distance_gate = smoothstep(dist_mean, Self::FLOW_DIST_MIN, Self::FLOW_DIST_FULL);
        let distance_consistency =
            (1.0 - (dist_stddev / dist_mean / 0.25).clamp(0.0, 1.0)).max(0.0);
        let flow_shape = shape * distance_gate * distance_consistency;

        let bpm = milliseconds_to_bpm(curr.adjusted_delta_time, None);
        let dense_rhythm = reverse_lerp(bpm, Self::FLOW_MIN_EFF_BPM, 360.0)
            * reverse_lerp(dist_mean, Self::FLOW_DIST_MIN, Self::FLOW_DIST_EXEMPT);

        let washing_machine = flow_shape
            * (1.0 - (angle_stddev / 0.12).clamp(0.0, 1.0))
            * (1.0 - (dist_stddev / dist_mean / 0.08).clamp(0.0, 1.0));

        // Very fast, evenly spaced flow is usually less aim-relevant under RX.
        // Preserve a strong penalty only for highly circular repetition.
        let fast_regular_flow = reverse_lerp(bpm, 280.0, 400.0)
            * distance_consistency
            * (1.0 - washing_machine);
        let speed_adjustment = (1.0 - 0.75 * fast_regular_flow).clamp(0.25, 1.0);

        let severity = (flow_shape.max(dense_rhythm * flow_shape) * speed_adjustment)
            .max(transition)
            .clamp(0.0, 1.0);

        Self::FLOW_MAX_NERF * severity
    }

    // Farm streak considers all farm-related nerfs (N/X, slop, cross-screen).
    // Flow aim is excluded.
    fn recent_farm_streak<'a>(
        curr: &'a OsuDifficultyObject<'a>,
        diff_objects: &'a [OsuDifficultyObject<'a>],
        window: usize,
    ) -> usize {
        let mut streak = 0;
        let mut current = Some(curr);

        while streak < window {
            if let Some(obj) = current {
                let nx_strength = detect_nx_pattern(obj, diff_objects, 4);

                // Angle consistency
                let (_, angle_stddev, angle_n) = windowed_angle_stats(obj, diff_objects, 4);

                // Distance consistency
                let (dist_mean, dist_stddev, dist_n) = windowed_dist_stats(obj, diff_objects, 4);

                // Velocity consistency
                let (vel_mean, vel_stddev, vel_n) = windowed_vel_stats(obj, diff_objects, 4);

                let slop_like = (angle_n >= 3 && angle_stddev < 0.30)
                    || (dist_n >= 3 && dist_mean > 0.0 && dist_stddev / dist_mean < 0.16)
                    || (vel_n >= 3 && vel_mean > 0.0 && vel_stddev / vel_mean < 0.16);

                // Cross-screen constant distance
                let cross_like = {
                    let curr_d = obj.lazy_jump_dist;
                    if let Some(prev) = obj.previous(0, diff_objects) {
                        let prev_d = prev.lazy_jump_dist;
                        let max_d = curr_d.max(prev_d);
                        if max_d > 80.0 {
                            let change_ratio = (max_d - prev_d.min(curr_d)) / max_d;
                            change_ratio < 0.20
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                let is_farm = nx_strength > 0.07 || slop_like || cross_like;

                if is_farm {
                    streak += 1;
                    current = obj.previous(0, diff_objects);
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        streak
    }

    fn recent_no_followpoint_streak<'a>(
        curr: &'a OsuDifficultyObject<'a>,
        diff_objects: &'a [OsuDifficultyObject<'a>],
        window: usize,
    ) -> usize {
        let mut streak = 0;
        let mut current = Some(curr);

        while streak < window {
            if let Some(obj) = current {
                if obj.lazy_jump_dist <= Self::FOLLOW_POINT_DISTANCE {
                    streak += 1;
                    current = obj.previous(0, diff_objects);
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        streak
    }

    fn combine_farm_severity(nx_strength: f64, slop_severity: f64, cross_severity: f64) -> f64 {
        1.0 - (1.0 - nx_strength) * (1.0 - slop_severity) * (1.0 - cross_severity)
    }

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

        // ── Velocities ──────────────────────────────────────────────
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

        // ── Angle bonuses ───────────────────────────────────────────
        if let Some((curr_angle, last_angle)) = osu_curr_obj.angle.zip(osu_last_obj.angle) {
            let angle_bonus = curr_vel.min(prev_vel);

            if osu_curr_obj
                .adjusted_delta_time
                .max(osu_last_obj.adjusted_delta_time)
                < 1.25
                    * osu_curr_obj
                        .adjusted_delta_time
                        .min(osu_last_obj.adjusted_delta_time)
            {
                acute_angle_bonus = Self::calc_acute_angle_bonus(curr_angle);

                acute_angle_bonus *= 0.08
                    + 0.92
                        * (1.0
                            - f64::min(
                                acute_angle_bonus,
                                f64::powf(Self::calc_acute_angle_bonus(last_angle), 3.0),
                            ));

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

            let (_win_mean, win_stddev, win_n) =
                windowed_angle_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);

            let variance_factor = if win_n >= 3 {
                (win_stddev / 1.2).clamp(0.0, 1.0)
            } else {
                1.0
            };

            let rep_strength = 1.0 - variance_factor;
            let wide_rep_nerf = rep_strength * 0.25;

            wide_angle_bonus *= 
                angle_bonus 
                * smootherstep(osu_curr_obj.lazy_jump_dist, 0.0, f64::from(DIAMETER))
                * (1.0 - wide_rep_nerf).max(0.0);
            
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

            if let Some(osu_last_2_obj) = curr.previous(2, diff_objects) {
                let distance =
                    (osu_last_2_obj.base.stacked_pos() - osu_last_obj.base.stacked_pos()).length();
                if distance < 1.0 {
                    wide_angle_bonus *= 1.0 - 0.35 * f64::from(1.0 - distance);
                }
            }
        }

        // ── Velocity change bonus ───────────────────────────────────
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

        // ── Slider bonus with slow-slider taper ─────────────────────
        // Slider breakability check
        if osu_last_obj.base.is_slider() {
            if osu_last_obj.travel_dist > osu_curr_obj.circle_radius * 1.2 {
                let travel_vel = osu_last_obj.travel_dist / osu_last_obj.travel_time;
                slider_bonus = travel_vel;

                if travel_vel < Self::SLOW_SLIDER_VEL_FLOOR {
                    let ratio = (travel_vel / Self::SLOW_SLIDER_VEL_FLOOR).clamp(0.0, 1.0);
                    slider_bonus *= 0.55 + 0.45 * ratio;
                }
            }
        }

        // ── Combine ─────────────────────────────────────────────────
        aim_strain += wiggle_bonus * Self::WIGGLE_MULTIPLIER;
        aim_strain += vel_change_bonus * Self::VELOCITY_CHANGE_MULTIPLIER;

        aim_strain += (acute_angle_bonus * Self::ACUTE_ANGLE_MULTIPLIER)
            .max(wide_angle_bonus * Self::WIDE_ANGLE_MULTIPLIER);

        aim_strain *= osu_curr_obj.small_circle_bonus;

        if with_slider_travel_dist {
            aim_strain += slider_bonus * Self::SLIDER_MULTIPLIER;
        }

        // ═════════════════════════════════════════════════════════════
        // CC V3 RX-specific nerfs and boosts (post-combine)
        // ═════════════════════════════════════════════════════════════

        // No pp for pseudo-stacks
        if osu_curr_obj.lazy_jump_dist <= Self::PSEUDO_STACK_THRESHOLD {
            return 0.0;
        }

        let precision_scaler = (osu_curr_obj.circle_radius / 36.0).clamp(0.2, 1.0);

        // Stack-to-dense transitions are included in the combined flow nerf.

        let eff_bpm = 30_000.0 / osu_curr_obj.adjusted_delta_time;
        let no_followpoint_streak =
            Self::recent_no_followpoint_streak(osu_curr_obj, diff_objects, 6);
        let skip_farm_detection = no_followpoint_streak < 6;

        let mut cross_screen_nerf = 0.0;
        let flow_nerf = Self::combined_flow_nerf(osu_curr_obj, diff_objects);
        let flow_active = flow_nerf > 0.0;

        // ── N/X alternating pattern severity ─────────────────────────────
        let nx_severity = if !skip_farm_detection {
            let nx_strength = detect_nx_pattern(osu_curr_obj, diff_objects, ANGLE_WINDOW);
            if nx_strength > 0.05 {
                let (dist_mean, dist_stddev, dist_n) =
                    windowed_dist_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);

                let dist_cv = if dist_mean > 0.0 && dist_n >= 3 {
                    dist_stddev / dist_mean
                } else {
                    1.0
                };

                let dist_consistency = (1.0 - (dist_cv / 0.20).clamp(0.0, 1.0)).max(0.0);
                let bpm_fade = 1.0 - ((eff_bpm - 350.0) / 150.0).clamp(0.0, 1.0);

                nx_strength * dist_consistency * bpm_fade
            } else {
                0.0
            }
        } else {
            0.0
        };

        // ── Aim slop detection ──────────────────────────────────────
        let slop_severity = if !skip_farm_detection {
            let (_, angle_stddev, angle_n) =
                windowed_angle_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);
            let (dist_mean, dist_stddev, dist_n) =
                windowed_dist_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);
            let (vel_mean, vel_stddev, vel_n) =
                windowed_vel_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);

            if angle_n >= 4 && dist_n >= 4 && vel_n >= 4 {
                let angle_uniformity = (1.0 - (angle_stddev / 0.3).clamp(0.0, 1.0)).max(0.0);
                let dist_cv = if dist_mean > 0.0 { dist_stddev / dist_mean } else { 1.0 };
                let dist_uniformity = (1.0 - (dist_cv / 0.15).clamp(0.0, 1.0)).max(0.0);
                let vel_cv = if vel_mean > 0.0 { vel_stddev / vel_mean } else { 1.0 };
                let vel_uniformity = (1.0 - (vel_cv / 0.15).clamp(0.0, 1.0)).max(0.0);

                let slop_severity = angle_uniformity * dist_uniformity * vel_uniformity;
                let bpm_fade = 1.0 - ((eff_bpm - 400.0) / 150.0).clamp(0.0, 1.0);

                slop_severity * bpm_fade
            } else {
                0.0
            }
        } else {
            0.0
        };

        // ── Cross-screen constant-distance nerf ─────────────────────
        // Cross-screen strictly relies on N/X or Slop
        if !flow_active && !skip_farm_detection && osu_curr_obj.adjusted_delta_time >= Self::CONSTANT_DIST_BPM_STRAIN_TIME {
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
                    let dist_factor = 1.0
                        - ((max_d - 80.0) / (Self::EDGE_TO_EDGE_THRESHOLD - 80.0))
                            .clamp(0.0, 1.0);
                    let severity = (1.0 - (change_ratio / Self::CONSTANT_DIST_RATIO)) * dist_factor;
                    
                    let cross_pattern_offender = (nx_severity + slop_severity).clamp(0.0, 1.0);
                    cross_screen_nerf = 0.15 * severity * cross_pattern_offender; 
                }
            }
        }

        let mut tech_boost = 0.0;
        let mut hybrid_boost = 0.0;

        if !flow_active && !skip_farm_detection {
            let (_, angle_stddev, angle_n) =
                windowed_angle_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);
            let (vel_mean, vel_stddev, vel_n) =
                windowed_vel_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);
            let (dist_mean, dist_stddev, dist_n) = 
                windowed_dist_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);

            if angle_n >= 4 && vel_n >= 4 {
                let angle_variety = ((angle_stddev - 0.6) / 0.4).clamp(0.0, 1.0);
                let vel_cv = if vel_mean > 0.0 { vel_stddev / vel_mean } else { 0.0 };
                let vel_variety = ((vel_cv - 0.25) / 0.25).clamp(0.0, 1.0);
                let tech_signal = angle_variety * vel_variety;
                tech_boost = Self::TECH_MAX_BOOST * tech_signal;
            }

            // Hybrid Section Boost
            if dist_n >= 4 {
                let dist_cv = if dist_mean > 0.0 { dist_stddev / dist_mean } else { 0.0 };
                // High distance variance (mixed jumps/streams) and not just a single transition
                if dist_cv > 0.40 && dist_mean > 30.0 {
                    hybrid_boost = (dist_cv - 0.40).clamp(0.0, 1.0) * Self::HYBRID_MAX_BOOST;
                }
            }
        }

        let farm_severity = Self::combine_farm_severity(nx_severity, slop_severity, cross_screen_nerf / 0.15);
        let farm_nerf = (Self::FARM_MAX_NERF * farm_severity).clamp(0.0, Self::FARM_MAX_NERF);
        
        // Precision scales the repeat nerf
        let mut repeat_nerf_factor = Self::relax_repeat_nerf_radius_factor(osu_curr_obj.circle_radius);
        repeat_nerf_factor = 1.0 - ((1.0 - repeat_nerf_factor) * precision_scaler);
        let farm_nerf = farm_nerf * repeat_nerf_factor;

        let recent_farm = Self::recent_farm_streak(osu_curr_obj, diff_objects, 5);

        aim_strain *= 1.15 - farm_nerf;
        if !flow_active {
            // Delayed tech buff after farm + neutral pattern protection + overall cap
            let apply_tech = !(recent_farm >= 3 && farm_nerf > 0.12)
                && !Self::is_neutral_flow_pattern(osu_curr_obj, diff_objects)
                && !Self::is_stream_pattern(osu_curr_obj, diff_objects);

            if apply_tech {
                let total_boost = tech_boost.max(hybrid_boost);
                aim_strain *= (1.0 + total_boost).min(Self::TECH_OVERALL_CAP);
            }
        }

        aim_strain *= 1.0 - flow_nerf; // NOTE: Haven't tested new flow nerf so this may need to be decreased to 0.95 or so to avoid overweight flow aim.

        // ── Akat calibration ────────────────────────────────────────
        aim_strain *= Self::AIM_CALIBRATION;

        aim_strain
    }

    const fn calc_wide_angle_bonus(angle: f64) -> f64 {
        smoothstep(angle, f64::to_radians(40.0), f64::to_radians(140.0))
    }

    const fn calc_acute_angle_bonus(angle: f64) -> f64 {
        smoothstep(angle, f64::to_radians(140.0), f64::to_radians(40.0))
    }
}
