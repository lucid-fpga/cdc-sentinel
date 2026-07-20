# Do not edit -- APF timing-constraint template (synthesized fixture)
set_clock_groups -asynchronous \
  -group {ic|core_pll|inst clk} \
  -group {ic|audio_pll|inst clk} \
  -group {ic|sdram_pll|inst clk} \
  -group {ic|video_pll|inst clk}
