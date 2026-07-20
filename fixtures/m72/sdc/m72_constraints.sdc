# author-added timing (re-added what the blanket cut removed)
set_multicycle_path -from [get_clocks cpu_clk] -to [get_clocks cpu_clk] 2
set_input_delay  -clock [get_clocks clk_dram] 2.0 [get_ports {dram_dq[*]}]
set_output_delay -clock [get_clocks clk_dram] 1.5 [get_ports {dram_dq[*]}]
