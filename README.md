# CCV3: Rosu-based Combo Consistency

This system implements **CCV3** logic utilizing `rosu-pp` rather than `akatsuki-pp`. While initially conceived as a fallback for potential compatibility issues with the modified Akatsuki system, it has been promoted to the primary implementation due to `rosu`'s superior developer experience—despite the underlying logic being more sophisticated.

## Development Roadmap

- [ ] **Deterministic Fail Detection**
    - Implement a 100% accurate, high-fidelity fail detection system.
    - Replace the current Claude-based heuristic detection with robust, deterministic logic to eliminate the inaccuracies and false readings inherent to the LLM approach.

- [ ] **Judgment & Miss Rebalancing**
    - Increase the performance penalty for misses to heighten difficulty scaling.
    - **N50 Refactor:**
        - Assign the first N50/miss a weight of 1 `effective_miss`.
        - Implement AR-dependent scaling where the first 2–3 N50s are counted as full `effective_misses`.
        - Revert to standard N50 scaling formulas once this initial threshold is exceeded.

- [ ] **Combo Weighting Overhaul**
    - Apply more aggressive scaling to the combo ratio factor.
    - Significantly increase the penalty for broken combos to ensure the final PP output better reflects play consistency.

- [ ] **System Integration & Porting**
    - Port and calibrate this refined calculation logic over to the Akat-based PP system for cross-compatibility.

- [ ] **Aim Scaling & Consistency Calibration**
    - Recalibrate aim PP scaling to align more closely with the original CCV3 values.
    - **Context:** This adjustment is critical as `rosu-pp` applies a more aggressive baseline nerf to "slop" or "farm" patterns compared to `akatsuki-pp`. Without this recalibration, the cumulative nerfs inherited from the original Akatsuki-based system may result in excessive performance penalties.

- [ ] **[ALTERNATIVE] Adjust Aim Constants Like WIDE_ANGLE_MULTIPLIER to Calibrate Aim PP**
    - Less accurate & `WIGGLE_MULTIPLIER` isnt in the akatsuki aim evaluator increasing the chances of inaccuracy.
     
- [ ] **[POTENTIAL/LIKELY] Reworked Exponential Miss Decay (STD Only)**
    - **Proposed Logic:** Replace the current static, tiered exponent system (e.g., 1.5–2.4 based on miss count) with a curve that scales **exponentially based on existing misses and `map_max_combo`**.
    - **Dynamic Weighting:** The miss decay curve will utilize a base weight derived from mod/combo-based $p$ values, with the exponent increasing dynamically for every additional miss. This ensures fairer weighting by accounting for map length rather than using rigid thresholds.
    - **Final Performance Calibration:** Upon completion, apply a final multiplier based on accuracy and miss count (with a length-dependent buffer for long maps).
    - **Implementation:** This will either be calculated server-side post-process or scaled in real-time by tracking the maximum possible accuracy and current miss count throughout the play.
    - **Note** This keeps the original point of the pp system intact while adding better logic for misses.

