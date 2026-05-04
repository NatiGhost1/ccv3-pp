# CCV3: Rosu-based Combo Consistency

This system implements **CCV3** logic utilizing `rosu-pp` rather than `akatsuki-pp`. While initially conceived as a fallback for potential compatibility issues with the modified Akatsuki system, it has been promoted to the primary implementation due to `rosu`'s superior developer experience—despite the underlying logic being more sophisticated. CCV3 is a fork of [`rosu-pp`](https://github.com/MaxOhn/rosu-pp).

## Development Roadmap

## OSU!STD

### Judgment & Miss Rebalancing
- Harshen the performance penalty for misses to heighten difficulty scaling.
- **N50 Refactor:**
    - Assign the first N50 a weight of 1 `effective_miss`.
    - **AR-Dependent Thresholding:** On specific AR values, the first 2–3 N50s are processed as 1 `effective_miss` each.
    - Revert to standard N50 scaling formulas immediately once this specific 2–3 count threshold is exceeded.
    - **NOTE** it is not only AR dependent OD plays a huge factor in the amount of n50's processed as 1 `effective_miss`.

### Combo Weighting Overhaul
- Apply more aggressive scaling to the combo ratio factor.
- Significantly increase the penalty for broken combos to ensure the final PP output better reflects play consistency.

### System Integration & Porting
- Port and calibrate this refined calculation logic over to the Akat-based PP system for cross-compatibility.

## [IMPLEMENTED] 

### Aim Scaling & Consistency Calibration
- Recalibrate aim PP scaling to align more closely with original CCV3 values.
- **Context:** The original CCV3 system was built on `akatsuki-pp`, where aim values are significantly more overweight compared to modern `rosu` calculations. Because `rosu` applies a much harsher baseline nerf to "slop" and "farm" patterns, failing to recalibrate would result in an unintentional "double-nerf" when combined with CCV3's consistency logic.
- **Methodology:** Surgically rework the base aim evaluator to mirror the output characteristics of Akatsuki’s sine-styled evaluation. 
- **Preservation of Architecture:** Retain the modern `rosu` framework rather than reverting to legacy code. Apply iterative modifications to the evaluator until the resulting PP values are roughly equivalent to the original Akatsuki-based benchmarks.

### Reworked Exponential Miss Decay (Vanilla Only)
- **Algorithmic Refinement:** Replace the current static, tiered exponent system (where `miss_exp` jumps between 1.5 and 2.4 at fixed thresholds) with a continuous, dynamic curve.
- **Dynamic Scaling Logic:** Instead of arbitrary "steps" at 2, 4, or 14 misses, the new system calculates a fluid exponent that scales based on `misses` and `map_max_combo`. This ensures the penalty is mathematically proportional to map length and total play impact.
- **Continuous Evolution:** The miss decay curve will utilize a base weight derived from mod/combo-based $p$ values, with the exponent increasing dynamically for every additional miss. This preserves the core philosophy of the system while providing a more granular and fair consistency evaluation.
- **Final Calibration:** Apply a final multiplier based on accuracy and miss count (with a length-dependent buffer for long maps) either server-side post-process or via real-time scaling of maximum possible accuracy.

### Deterministic Fail Detection
- Implement a 100% accurate, high-fidelity fail detection system.
- Replace the current Claude-based heuristic detection with robust, deterministic logic to eliminate the inaccuracies and false readings inherent to the LLM approach.

## OSU!TAIKO

### Research OSU!TAIKO PP System
- Need to know how it works before I can do anything lol


