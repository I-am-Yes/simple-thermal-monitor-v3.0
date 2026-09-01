use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThermalReading {
    pub cpu_package: Option<f32>,
    pub p_core_0: Option<f32>,
    pub gpu: Option<f32>,
    pub ssd: Option<f32>,
}

pub fn read_temperatures() -> ThermalReading {
    #[cfg(windows)]
    {
        windows_impl::read()
    }
    #[cfg(not(windows))]
    {
        ThermalReading::default()
    }
}

pub(crate) fn plausible(temp: f32) -> bool {
    (1.0..=125.0).contains(&temp)
}

pub(crate) fn round_tenth(temp: f32) -> f32 {
    (temp * 10.0).round() / 10.0
}

/// ACPI CurrentTemperature is tenths of Kelvin (e.g. 3010).
/// Perf `Temperature` is Kelvin (e.g. 301). HighPrecisionTemperature is tenths of Kelvin.
fn thermal_raw_to_c(raw: u32) -> Option<f32> {
    let kelvin = if raw >= 1000 {
        raw as f32 / 10.0
    } else {
        raw as f32
    };
    let celsius = kelvin - 273.15;
    plausible(celsius).then_some(round_tenth(celsius))
}

#[cfg(windows)]
mod windows_impl {
    use super::{plausible, round_tenth, thermal_raw_to_c, ThermalReading};
    use serde::Deserialize;

    pub fn read() -> ThermalReading {
        std::thread::Builder::new()
            .name("thermal-read".into())
            .spawn(|| read_on_thread())
            .ok()
            .and_then(|handle| handle.join().ok())
            .unwrap_or_else(|| crate::lhm::latest().unwrap_or_default())
    }

    fn read_on_thread() -> ThermalReading {
        let lhm = crate::lhm::latest().unwrap_or_default();
        if complete(&lhm) {
            return lhm;
        }

        let zones = read_thermal_zones();
        let (zone_cpu, zone_pcore, zone_gpu) = map_cpu_zones(&zones);

        ThermalReading {
            cpu_package: lhm.cpu_package.or(zone_cpu),
            p_core_0: lhm.p_core_0.or(zone_pcore),
            gpu: lhm.gpu.or_else(read_gpu_nvml).or(zone_gpu),
            ssd: lhm.ssd.or_else(read_ssd_temperature),
        }
    }

    fn complete(reading: &ThermalReading) -> bool {
        reading.cpu_package.is_some()
            && reading.p_core_0.is_some()
            && reading.gpu.is_some()
            && reading.ssd.is_some()
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename = "MSAcpi_ThermalZoneTemperature")]
    struct AcpiThermalZone {
        #[serde(rename = "CurrentTemperature")]
        current_temperature: Option<u32>,
        #[serde(rename = "InstanceName")]
        instance_name: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct PerfThermalZone {
        #[serde(rename = "Name")]
        name: Option<String>,
        #[serde(rename = "Temperature")]
        temperature: Option<u32>,
        #[serde(rename = "HighPrecisionTemperature")]
        high_precision_temperature: Option<u32>,
    }

    struct NamedTemp {
        name: String,
        temp: f32,
    }

    fn push_zone(zones: &mut Vec<NamedTemp>, name: String, temp: f32) {
        if zones.iter().any(|z| z.name.eq_ignore_ascii_case(&name)) {
            return;
        }
        zones.push(NamedTemp { name, temp });
    }

    fn read_thermal_zones() -> Vec<NamedTemp> {
        let mut zones = Vec::new();
        let Ok(com) = wmi::COMLibrary::new() else {
            return zones;
        };

        if let Ok(conn) = wmi::WMIConnection::with_namespace_path(r"root\wmi", com) {
            if let Ok(results) = conn.query::<AcpiThermalZone>() {
                for zone in results {
                    let Some(raw) = zone.current_temperature else {
                        continue;
                    };
                    let Some(temp) = thermal_raw_to_c(raw) else {
                        continue;
                    };
                    push_zone(
                        &mut zones,
                        zone.instance_name.unwrap_or_default(),
                        temp,
                    );
                }
            }
        }

        if let Ok(com) = wmi::COMLibrary::new() {
            if let Ok(conn) = wmi::WMIConnection::new(com) {
                if let Ok(results) = conn.raw_query::<PerfThermalZone>(
                    "SELECT Name, Temperature, HighPrecisionTemperature FROM Win32_PerfFormattedData_Counters_ThermalZoneInformation",
                ) {
                    for zone in results {
                        let temp = zone
                            .high_precision_temperature
                            .and_then(thermal_raw_to_c)
                            .or_else(|| zone.temperature.and_then(thermal_raw_to_c));
                        let Some(temp) = temp else {
                            continue;
                        };
                        push_zone(&mut zones, zone.name.unwrap_or_default(), temp);
                    }
                } else if let Ok(results) = conn.raw_query::<PerfThermalZone>(
                    "SELECT Name, Temperature FROM Win32_PerfFormattedData_Counters_ThermalZoneInformation",
                ) {
                    for zone in results {
                        let Some(temp) = zone.temperature.and_then(thermal_raw_to_c) else {
                            continue;
                        };
                        push_zone(&mut zones, zone.name.unwrap_or_default(), temp);
                    }
                }
            }
        }

        zones
    }

    fn map_cpu_zones(zones: &[NamedTemp]) -> (Option<f32>, Option<f32>, Option<f32>) {
        let gpu = zones
            .iter()
            .find(|z| {
                let n = z.name.to_ascii_uppercase();
                n.contains("GPU") || n.contains("GFX") || n.contains("VIDEO")
            })
            .map(|z| z.temp);

        let cpu_zones: Vec<&NamedTemp> = zones
            .iter()
            .filter(|z| {
                let n = z.name.to_ascii_uppercase();
                !n.contains("GPU")
                    && !n.contains("GFX")
                    && !n.contains("SSD")
                    && !n.contains("DISK")
                    && !n.contains("NVME")
                    && !n.contains("HDD")
            })
            .collect();

        let cpu_package = cpu_zones
            .iter()
            .find(|z| {
                let n = z.name.to_ascii_uppercase();
                n.contains("CPU") || n.contains("PKG") || n.contains("PACKAGE")
            })
            .or_else(|| cpu_zones.first())
            .map(|z| z.temp);

        let p_core_0 = cpu_zones
            .iter()
            .find(|z| {
                let n = z.name.to_ascii_uppercase();
                n.contains("CORE") || n.contains("PCORE") || n.contains("P-CORE")
            })
            .copied()
            .filter(|z| Some(z.temp) != cpu_package)
            .or_else(|| {
                cpu_zones
                    .get(1)
                    .copied()
                    .filter(|z| Some(z.temp) != cpu_package)
            })
            .map(|z| z.temp)
            .or(cpu_package);

        (cpu_package, p_core_0, gpu)
    }

    fn read_gpu_nvml() -> Option<f32> {
        let nvml = nvml_wrapper::Nvml::init().ok()?;
        let count = nvml.device_count().ok()?;
        let mut best: Option<f32> = None;
        for index in 0..count {
            let Ok(device) = nvml.device_by_index(index) else {
                continue;
            };
            let Ok(temp) =
                device.temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu)
            else {
                continue;
            };
            let temp = temp as f32;
            if plausible(temp) {
                best = Some(best.map_or(temp, |current| current.max(temp)));
            }
        }
        best
    }

    fn read_ssd_temperature() -> Option<f32> {
        let mut best: Option<f32> = None;
        for index in 0..16 {
            let path = format!(r"\\.\PhysicalDrive{index}");
            if let Some(temp) = query_drive_temperature(&path) {
                best = Some(best.map_or(temp, |current| current.max(temp)));
            }
        }
        best
    }

    fn open_physical_drive(wide: &[u16]) -> Option<windows::Win32::Foundation::HANDLE> {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE};
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };

        let accesses = if crate::elevate::is_elevated() {
            [GENERIC_READ.0, 0]
        } else {
            [0, GENERIC_READ.0]
        };

        for access in accesses {
            let handle = unsafe {
                CreateFileW(
                    PCWSTR(wide.as_ptr()),
                    access,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                )
            };
            if let Ok(handle) = handle {
                if handle != INVALID_HANDLE_VALUE && !handle.is_invalid() {
                    return Some(handle);
                }
            }
        }
        None
    }

    fn query_drive_temperature(path: &str) -> Option<f32> {
        use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows::Win32::System::IO::DeviceIoControl;

        const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D1400;
        const STORAGE_DEVICE_TEMPERATURE_PROPERTY: u32 = 52;
        const PROPERTY_STANDARD_QUERY: u32 = 0;

        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = open_physical_drive(&wide)?;

        if handle == INVALID_HANDLE_VALUE || handle.is_invalid() {
            return None;
        }

        let mut query = [0u8; 12];
        query[0..4].copy_from_slice(&STORAGE_DEVICE_TEMPERATURE_PROPERTY.to_le_bytes());
        query[4..8].copy_from_slice(&PROPERTY_STANDARD_QUERY.to_le_bytes());

        let mut output = [0u8; 512];
        let mut returned = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                Some(query.as_ptr().cast()),
                query.len() as u32,
                Some(output.as_mut_ptr().cast()),
                output.len() as u32,
                Some(&mut returned),
                None,
            )
        };

        unsafe {
            let _ = CloseHandle(handle);
        }

        if ok.is_err() || returned < 16 {
            return None;
        }

        let mut info_count = u16::from_le_bytes(output[12..14].try_into().ok()?) as usize;
        if info_count == 0 && returned >= 24 {
            info_count = u32::from_le_bytes(output[20..24].try_into().ok()?) as usize;
        }
        info_count = info_count.min(16);

        let mut composite = None;
        let mut found = None;
        for i in 0..info_count {
            let offset = 24 + i * 16;
            if offset + 4 > returned as usize || offset + 4 > output.len() {
                break;
            }
            let index = u16::from_le_bytes(output[offset..offset + 2].try_into().ok()?);
            let raw = i16::from_le_bytes(output[offset + 2..offset + 4].try_into().ok()?);
            if raw == i16::MIN {
                continue;
            }
            let temp = raw as f32;
            if !plausible(temp) {
                continue;
            }
            let temp = round_tenth(temp);
            if index == 0 {
                composite = Some(temp);
            }
            found = Some(found.map_or(temp, |current: f32| current.max(temp)));
        }
        composite.or(found)
    }
}

#[cfg(test)]
mod tests {
    use super::thermal_raw_to_c;

    #[test]
    fn kelvin_perf_counter() {
        assert_eq!(thermal_raw_to_c(301), Some(27.9));
    }

    #[test]
    fn tenths_kelvin_acpi() {
        assert_eq!(thermal_raw_to_c(3010), Some(27.9));
    }

    #[cfg(windows)]
    #[test]
    fn reads_cpu_or_ssd_without_admin() {
        let reading = crate::temps::read_temperatures();
        eprintln!("reading: {reading:?}");
        assert!(
            reading.cpu_package.is_some(),
            "CPU package should come from thermal zone perf counters"
        );
        assert!(
            reading.ssd.is_some(),
            "SSD should come from StorageDeviceTemperatureProperty"
        );
    }
}
