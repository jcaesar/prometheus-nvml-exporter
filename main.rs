use nvml_wrapper::{Device, Nvml};
use prometheus::{
    register_gauge_vec, register_int_counter_vec, register_int_gauge_vec, GaugeVec, IntCounterVec,
    IntGaugeVec,
};
use std::cmp;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

#[derive(clap::Parser)]
#[clap(author, version, about)]
struct Opts {
    /// Listen address/port
    #[structopt(short = 'l', long = "listen", default_value = "[::]:9144")]
    listen: SocketAddr,
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

static GPU_LABELS: [&str; 3] = ["uuid", "name", "pci"];
lazy_static::lazy_static! {
    static ref MEMORY_FREE: IntGaugeVec =
        register_int_gauge_vec!("nvml_memory_free_bytes", "Free Memory", &GPU_LABELS).unwrap();
    static ref MEMORY_USED: IntGaugeVec =
        register_int_gauge_vec!("nvml_memory_used_bytes", "Used Memory", &GPU_LABELS).unwrap();
    static ref MEMORY_TOTAL: IntGaugeVec =
        register_int_gauge_vec!("nvml_memory_total_bytes", "Total Memory", &GPU_LABELS).unwrap();
    static ref FAN_SPEED: GaugeVec = register_gauge_vec!(
        "nvml_fan_speed",
        "Fan speed (0-1)",
        &[&GPU_LABELS[..], &["fan"][..]].concat()
    )
    .unwrap();
    static ref TEMPERATURE: GaugeVec =
        register_gauge_vec!("nvml_temp", "Temperature degC", &GPU_LABELS).unwrap();
    static ref PERFORMANCE_STATE: IntGaugeVec = register_int_gauge_vec!(
        "nvml_performance_state",
        "Performance State (between 15 (low) and 0 (high))",
        &GPU_LABELS
    )
    .unwrap();
    static ref POWER_USAGE: IntGaugeVec = register_int_gauge_vec!(
        "nvml_power_usage_current_mw",
        "Current power usage (mW)",
        &GPU_LABELS
    )
    .unwrap();
    static ref POWER_MAX: IntGaugeVec = register_int_gauge_vec!(
        "nvml_power_usage_max_mw",
        "Enforced power limit (mW)",
        &GPU_LABELS
    )
    .unwrap();
    static ref ENERGY_USED: IntCounterVec = register_int_counter_vec!(
        "nvml_power_used_total_mj",
        "Energy used in total",
        &GPU_LABELS
    )
    .unwrap();
    static ref PCI_REPLAY: IntCounterVec =
        register_int_counter_vec!("nvml_pci_replay", "Energy used in total", &GPU_LABELS).unwrap();
}

struct MetricDevice<'a> {
    device: Device<'a>,
    labels: [String; 3],
    fan_count: u32,
}

impl MetricDevice<'_> {
    fn new(device: Device) -> Result<MetricDevice<'_>> {
        let mut i: u32 = 0;
        Ok(MetricDevice {
            fan_count: loop {
                if i > 10_000 || device.fan_speed(i).is_err() {
                    break i;
                };
                i += 1;
            },
            labels: [device.uuid()?, device.name()?, device.pci_info()?.bus_id],
            device,
        })
    }
    fn labels(&self) -> Vec<&str> {
        self.labels.iter().map(|x| x.as_ref()).collect()
    }
    fn performance_state(&self) -> Result<i64> {
        use nvml_wrapper::enum_wrappers::device::PerformanceState::*;
        Ok(match self.device.performance_state()? {
            Zero => 0,
            One => 1,
            Two => 2,
            Three => 3,
            Four => 4,
            Five => 5,
            Six => 6,
            Seven => 7,
            Eight => 8,
            Nine => 9,
            Ten => 10,
            Eleven => 11,
            Twelve => 12,
            Thirteen => 13,
            Fourteen => 14,
            Fifteen => 15,
            Unknown => -1,
        })
    }
    fn update(&self) -> Result<()> {
        let meminfo = self.device.memory_info()?;
        MEMORY_FREE
            .get_metric_with_label_values(&self.labels())?
            .set(meminfo.free.try_into()?);
        MEMORY_USED
            .get_metric_with_label_values(&self.labels())?
            .set(meminfo.used.try_into()?);
        MEMORY_TOTAL
            .get_metric_with_label_values(&self.labels())?
            .set(meminfo.total.try_into()?);
        for i in 0..self.fan_count {
            FAN_SPEED
                .get_metric_with_label_values(
                    &[&self.labels()[..], &[format!("{}", i).as_ref()][..]].concat(),
                )?
                .set(self.device.fan_speed(i)? as f64 / 100.);
        }
        TEMPERATURE
            .get_metric_with_label_values(&self.labels())?
            .set(
                self.device
                    .temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu)?
                    as f64,
            );
        PERFORMANCE_STATE
            .get_metric_with_label_values(&self.labels())?
            .set(self.performance_state()?);
        POWER_USAGE
            .get_metric_with_label_values(&self.labels())?
            .set(self.device.power_usage()? as i64);
        POWER_MAX
            .get_metric_with_label_values(&self.labels())?
            .set(self.device.enforced_power_limit()? as i64);
        let energy_prev = ENERGY_USED
            .get_metric_with_label_values(&self.labels())?
            .get();
        let energy_current: u64 = self.device.total_energy_consumption()?.try_into()?;
        ENERGY_USED
            .get_metric_with_label_values(&self.labels())?
            .inc_by(energy_current - energy_prev);
        let replay_prev = PCI_REPLAY
            .get_metric_with_label_values(&self.labels())?
            .get();
        let replay_current: u64 = self.device.pcie_replay_counter()?.try_into()?;
        PCI_REPLAY
            .get_metric_with_label_values(&self.labels())?
            .inc_by(replay_current - replay_prev);
        Ok(())
    }
}

fn main() -> Result<()> {
    let opts: Opts = clap::Parser::parse();

    let exporter = prometheus_exporter::start(opts.listen)?;

    let mut lastdevices = 0;
    let mut refresh_interval = Duration::from_secs(30);

    loop {
        let nvml = Nvml::init()?;
        let devices = (0..(nvml.device_count()?))
            .map(|idx| nvml.device_by_index(idx))
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .map(MetricDevice::new)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        refresh_interval = match lastdevices == devices.len() {
            false => Duration::from_secs(30),
            true => cmp::min(refresh_interval * 2, Duration::from_secs(3600)),
        };
        lastdevices = devices.len();
        let nextupdate = Instant::now() + refresh_interval;

        while Instant::now() < nextupdate {
            let _update_guard = exporter.wait_request();
            for dev in &devices {
                dev.update()?;
            }
        }
    }
}
