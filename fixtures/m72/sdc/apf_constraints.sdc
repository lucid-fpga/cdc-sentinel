# Do not edit -- APF timing-constraint template (synthesized fixture)
set_clock_groups -asynchronous \
  -group {ic|mp1|core_pll|inst clk} \
  -group {ic|audio_pll|inst clk} \
  -group {ic|video_pll|video_pll_inst|altera_pll_i clk} \
  -group {ic|sdram_pll|inst clk}
