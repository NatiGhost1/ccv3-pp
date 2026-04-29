# rosu based combo consistency
CCV3 based off of rosu-pp instead of akatsuki-pp.
Originally made in case the akatsuki based pp system isnt compatible.
Now it serves as the main CCV3 pp system since rosu is easier to work with (but the system itself is more complex imo)

# TODO

- [ ] **Advanced Fail Detection**
    - Implement a 100% accurate, no-fail detection system.
    - Replace current logic (Claude-based) with a more robust, deterministic check.

- [ ] **Miss/Judgment Rebalancing**
    - Harshen the penalty for misses.
    - Rework N50s to function as misses:
        - The first N50/miss always equals 1 `effective_miss`.
        - Depending on the Map AR, the first 2–3 N50s scale as 1 `effective_miss`.
        - **Transition to original N50 scaling** immediately following this initial threshold.

- [ ] **Combo Scaling**
    - Increase the severity of the combo ratio factor.
    - Make combo-based scaling significantly harsher on the final score.

- [ ] **System Integration**
    - Port the updated logic over to the Akat-based PP system.

