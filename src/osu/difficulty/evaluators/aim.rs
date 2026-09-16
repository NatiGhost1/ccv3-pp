use crate::{
    any::difficulty::object::IDifficultyObject,
    osu::difficulty::object::OsuDifficultyObject,
    util::{
        difficulty::{milliseconds_to_bpm, reverse_lerp, smoothstep_aim, smootherstep_aim},
        float_ext::FloatExt,
    },
};

pub struct AimEvaluator;

// ─── Windowed statistics helpers ───────────────

const ANGLE_WINDOW: usize = 8;

/// Traverses up to `window` past objects to calculate the mean angle, 
/// standard deviation (variance sqrt), and valid sample count.
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
        return (0.0, 0.0, n); // Requires at least 3 angles for meaningful variance
    }
    let mean: f64 = angles.iter().sum::<f64>() / n as f64;
    let variance: f64 = angles.iter().map(|a| (a - mean).powi(2)).sum::<f64>() / n as f64;
    (mean, variance.sqrt(), n)
}

/// Gathers lazy jump distances over a sliding window to calculate average distance 
/// and standard deviation across recent hit objects.
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

/// Evaluates velocity (distance / adjusted_delta_time) across recent objects 
/// to track speed consistency over the sliding window.
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

/// Measures how predictable and structured a flow section is.
/// Smooth, repetitive flow aim gets penalized, but complex technical sections 
/// are protected by the hard-pattern guard thresholds.
fn flow_pattern_predictability(
    angle_mean: f64,
    angle_stddev: f64,
    dist_mean: f64,
    dist_stddev: f64,
    vel_mean: f64,
    vel_stddev: f64,
) -> f64 {
    // Flow requires wide, wide-sweeping angles (> 90 degrees / FRAC_PI_2).
    if angle_mean <= std::f64::consts::FRAC_PI_2 {
        return 0.0;
    }

    let angle_consistency = (1.0 - (angle_stddev / 0.18).clamp(0.0, 1.0)).max(0.0);

    // Coefficient of variation (CV = stddev / mean) measures relative fluctuation
    let dist_cv = if dist_mean > 0.0 {
        dist_stddev / dist_mean
    } else {
        1.0
    };
    let vel_cv = if vel_mean > 0.0 {
        vel_stddev / vel_mean
    } else {
        1.0
    };

    // HARD-PATTERN GUARD: If angle, distance, or velocity varies too drastically, 
    // it's a technical section rather than predictable farm flow—skip the nerf.
    if angle_stddev > 0.20 || dist_cv > 0.22 || vel_cv > 0.18 {
        return 0.0;
    }

    let dist_uniformity = (1.0 - (dist_cv / 0.16).clamp(0.0, 1.0)).max(0.0);
    let vel_uniformity = (1.0 - (vel_cv / 0.14).clamp(0.0, 1.0)).max(0.0);
    let flow_shape = smoothstep_aim(angle_mean, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);

    // Weighted combination of consistency factors
    let predictability = (angle_consistency * 0.5 + dist_uniformity * 0.3 + vel_uniformity * 0.2)
        .clamp(0.0, 1.0);

    predictability * flow_shape
}

impl AimEvaluator {
    const WIDE_ANGLE_MULTIPLIER: f64 = 1.5;
    const ACUTE_ANGLE_MULTIPLIER: f64 = 2.55;
    const SLIDER_MULTIPLIER: f64 = 1.35;
    const VELOCITY_CHANGE_MULTIPLIER: f64 = 0.75;
    const WIGGLE_MULTIPLIER: f64 = 1.02;

    const AIM_CALIBRATION: f64 = 1.0; 

    #[expect(clippy::too_many_lines, reason = "staying in-sync with lazer")]
    pub fn evaluate_diff_of<'a>(
        curr: &'a OsuDifficultyObject<'a>,
        diff_objects: &'a [OsuDifficultyObject<'a>],
        with_slider_travel_dist: bool,
    ) -> f64 {
        let osu_curr_obj = curr;

        // Fetch previous objects; ignore spinners as they don't involve aim strain.
        let Some((osu_last_last_obj, osu_last_obj)) = curr
            .previous(1, diff_objects)
            .zip(curr.previous(0, diff_objects))
            .filter(|(_, last)| !(curr.base.is_spinner() || last.base.is_spinner()))
        else {
            return 0.0;
        };

        #[expect(clippy::items_after_statements, reason = "staying in-sync with lazer")]
        const RADIUS: i32 = OsuDifficultyObject::NORMALIZED_RADIUS;
        #[expect(clippy::items_after_statements, reason = "staying in-sync with lazer")]
        const DIAMETER: i32 = OsuDifficultyObject::NORMALIZED_DIAMETER;

        // Base jump velocity for the current object
        let mut curr_vel = osu_curr_obj.lazy_jump_dist / osu_curr_obj.adjusted_delta_time;

        // If previous object is a slider, blend movement velocity with slider travel velocity.
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

        // ── Angle Strain & Repetition Penalties ──────────────────────────────
        if let Some((curr_angle, last_angle)) = osu_curr_obj.angle.zip(osu_last_obj.angle) {
            let angle_bonus = curr_vel.min(prev_vel);

            // Acute angle calculation applies only if object rhythms (delta times) are similar.
            if osu_curr_obj
                .adjusted_delta_time
                .max(osu_last_obj.adjusted_delta_time)
                < 1.25
                    * osu_curr_obj
                        .adjusted_delta_time
                        .min(osu_last_obj.adjusted_delta_time)
            {
                acute_angle_bonus = Self::calc_acute_angle_bonus(curr_angle);

                // Diminishing returns for repeating acute angles
                acute_angle_bonus *= 0.08
                    + 0.92
                        * (1.0
                            - f64::min(
                                acute_angle_bonus,
                                f64::powf(Self::calc_acute_angle_bonus(last_angle), 3.0),
                            ));

                // Scale acute angles higher for rapid BPM (>300-400) and wider spacing
                acute_angle_bonus *= angle_bonus
                    * smootherstep_aim(
                        milliseconds_to_bpm(osu_curr_obj.adjusted_delta_time, Some(2)),
                        300.0,
                        400.0,
                    )
                    * smootherstep_aim(
                        osu_curr_obj.lazy_jump_dist,
                        f64::from(DIAMETER),
                        f64::from(DIAMETER * 2),
                    );
            }

            wide_angle_bonus = Self::calc_wide_angle_bonus(curr_angle);

            // Compute statistical variance across past window
            let (angle_mean, angle_stddev, angle_n) =
                windowed_angle_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);
            let (vel_mean, vel_stddev, vel_n) =
                windowed_vel_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);

            let variance_factor = if angle_n >= 3 {
                (angle_stddev / 1.2).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let rep_strength = 1.0 - variance_factor;

            let wide_rep_raw = wide_angle_bonus
                .min(Self::calc_wide_angle_bonus(last_angle).powf(3.0));
            
            // Base penalty for repetitive wide movements
            let mut wide_penalty = rep_strength * 0.7 + wide_rep_raw * 0.3;

            // Apply advanced flow predictability nerf if pattern is highly structured
            if angle_n >= 4 && vel_n >= 4 {
                let (dist_mean, dist_stddev, dist_n) =
                    windowed_dist_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);

                if dist_n >= 4 {
                    let predictability = flow_pattern_predictability(
                        angle_mean,
                        angle_stddev,
                        dist_mean,
                        dist_stddev,
                        vel_mean,
                        vel_stddev,
                    );

                    if predictability > 0.0 {
                        let advanced_flow_nerf = 0.60 * predictability;
                        wide_penalty += advanced_flow_nerf;
                    }
                }
            }

            wide_angle_bonus *= angle_bonus
                * smoothstep_aim(osu_curr_obj.lazy_jump_dist, 0.0, f64::from(DIAMETER))
                * ((1.0 - wide_penalty).max(0.0));

            let acute_rep_raw = acute_angle_bonus
                .min(Self::calc_acute_angle_bonus(last_angle).powf(3.0));
            
            let acute_penalty = rep_strength * 0.5 + acute_rep_raw * 0.5;
            acute_angle_bonus *= (0.5 + 0.5 * (1.0 - acute_penalty)).max(0.0);

            // ── Wiggle Bonus ──────────────────────────────────────────────────
            // Buffs high-speed, small micro-jumps/wiggles between tight circles.
            wiggle_bonus = angle_bonus
                * smoothstep_aim(
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
                * smootherstep_aim(curr_angle, f64::to_radians(110.0), f64::to_radians(60.0))
                * smootherstep_aim(
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
                * smoothstep_aim(last_angle, f64::to_radians(110.0), f64::to_radians(60.0));

            // Stacking adjustment: nerf bonus if notes sit virtually on top of each other
            if let Some(osu_last_2_obj) = curr.previous(2, diff_objects) {
                let distance =
                    (osu_last_2_obj.base.stacked_pos() - osu_last_obj.base.stacked_pos()).length();

                if distance < 1.0 {
                    wide_angle_bonus *= 1.0 - 0.35 * f64::from(1.0 - distance);
                }
            }
        }

        // ── Velocity Change / Overlap Bonus ──────────────────────────────────
        // Rewards strain when transitioning speeds or handling overlapping jumps.
        if prev_vel.max(curr_vel).not_eq(0.0) {
            prev_vel = (osu_last_obj.lazy_jump_dist + osu_last_last_obj.travel_dist)
                / osu_last_obj.adjusted_delta_time;
            curr_vel = (osu_curr_obj.lazy_jump_dist + osu_last_obj.travel_dist)
                / osu_curr_obj.adjusted_delta_time;

            let dist_ratio = smoothstep_aim(
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

            let bonus_base = (osu_curr_obj.adjusted_delta_time)
                .min(osu_last_obj.adjusted_delta_time)
                / (osu_curr_obj.adjusted_delta_time).max(osu_last_obj.adjusted_delta_time);
            vel_change_bonus *= bonus_base.powf(2.0);
        }

        // Add slider movement strain if applicable
        if osu_last_obj.base.is_slider() {
            slider_bonus = osu_last_obj.travel_dist / osu_last_obj.travel_time;
        }

        // Combine bonuses into final aim strain
        aim_strain += wiggle_bonus * Self::WIGGLE_MULTIPLIER;
        aim_strain += vel_change_bonus * Self::VELOCITY_CHANGE_MULTIPLIER;

        aim_strain += (acute_angle_bonus * Self::ACUTE_ANGLE_MULTIPLIER)
            .max(wide_angle_bonus * Self::WIDE_ANGLE_MULTIPLIER);

        aim_strain *= osu_curr_obj.small_circle_bonus; // Boost small CS / smaller hit circles

        if with_slider_travel_dist {
            aim_strain += slider_bonus * Self::SLIDER_MULTIPLIER;
        }

        // ── Unified Advanced Farm & Cross-Screen Nerf ────────────────────────
        // Checks structural uniformity across angle, distance, and speed. Simple 1-2 
        // cross-screen farm jumps (e.g. N or X patterns) get heavily reduced.
        let mut unified_nerf = 0.0;
        {
            let (_, angle_stddev, angle_n) =
                windowed_angle_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);
            let (dist_mean, dist_stddev, dist_n) =
                windowed_dist_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);
            let (vel_mean, vel_stddev, vel_n) =
                windowed_vel_stats(osu_curr_obj, diff_objects, ANGLE_WINDOW);

            if angle_n >= 4 && dist_n >= 4 && vel_n >= 4 {
                let angle_uniformity = (1.0 - (angle_stddev / 0.4).clamp(0.0, 1.0)).max(0.0);
                
                let dist_cv = if dist_mean > 0.0 { dist_stddev / dist_mean } else { 1.0 };
                let dist_uniformity = (1.0 - (dist_cv / 0.25).clamp(0.0, 1.0)).max(0.0);

                let vel_cv = if vel_mean > 0.0 { vel_stddev / vel_mean } else { 1.0 };
                let vel_uniformity = (1.0 - (vel_cv / 0.20).clamp(0.0, 1.0)).max(0.0);

                // High pattern slop = extremely predictable geometry across all metrics
                let pattern_slop = angle_uniformity * dist_uniformity * vel_uniformity;

                // Scale nerf up if jump distance spans far across the screen (>= 3x diameter)
                let cross_screen_factor = (dist_mean / f64::from(DIAMETER * 3)).clamp(0.0, 1.0);
                
                let base_nerf_strength = 0.15;
                unified_nerf = base_nerf_strength * pattern_slop * (1.0 + cross_screen_factor * 0.5);
            }
        }

        aim_strain *= (1.0 - unified_nerf).max(0.0);
        aim_strain *= Self::AIM_CALIBRATION;

        aim_strain
    }

    /// Evaluates wide angle scaling (40 deg to 140 deg)
    fn calc_wide_angle_bonus(angle: f64) -> f64 {
        smoothstep_aim(angle, f64::to_radians(40.0), f64::to_radians(140.0))
    }

    /// Evaluates acute angle scaling (140 deg down to 40 deg sharp turn)
    fn calc_acute_angle_bonus(angle: f64) -> f64 {
        smoothstep_aim(angle, f64::to_radians(140.0), f64::to_radians(40.0))
    }
}
