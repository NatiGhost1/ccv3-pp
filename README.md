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
