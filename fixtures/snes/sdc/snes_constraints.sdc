# author-added timing
set_multicycle_path -from [get_clocks mclk] -to [get_clocks mclk] 2
set_false_path -from [get_clocks clk_74a] -to [get_clocks mclk]
# NOTE: cross-core copied path below (a dead constraint Lint A v1 does not
# yet catch -- non-*_pll hierarchy token; recorded as a known limitation)
set_false_path -through {ic|nes|sdram|*}
