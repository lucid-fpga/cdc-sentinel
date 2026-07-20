set_multicycle_path -from [get_clocks mclk] -to [get_clocks mclk] 2
set_false_path -from [get_clocks clk_74a] -to [get_clocks mclk]
