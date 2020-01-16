# Entirely insufficient nvml / nvidia graphics card metrics exporter

Currently exports the following metrics
```
nvml_fan_speed
nvml_memory_free_bytes
nvml_memory_total_bytes
nvml_memory_used_bytes
nvml_pci_replay
nvml_performance_state
nvml_power_usage_current_mw
nvml_power_usage_max_mw
nvml_power_used_total_mj
```
with labesl like `{name="GeForce RTX 2080",pci="00000000:0A:00.0",uuid="GPU-4be17369-5fd4-6000-889b-9da3c63e45f3"}`

### Todo
* Per process metrics (as in nvidia-smi)
* More efficient format when queried by prometheus (compression / protobuf)

