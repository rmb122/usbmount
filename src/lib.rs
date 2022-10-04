use lazy_static::lazy_static;
use regex::Regex;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, BufRead},
    path::Path,
};
use udev::Device;
extern crate udev;

#[derive(Debug)]
struct MountInfo {
    info: HashMap<String, Vec<String>>,
}

impl MountInfo {
    fn parse() -> Self {
        let mut info: HashMap<String, Vec<String>> = HashMap::new();

        let mount_file =
            File::open("/proc/self/mountinfo").expect("/proc/self/mountinfo open failed");
        let reader = io::BufReader::new(mount_file);

        for line in reader.lines() {
            // <id> <parent_id> <major>:<minor> <root> <mount_point> <mount_options> <optional tags> "-" <fstype> <mount souce> <super options>
            // from https://github.com/GuillemCastro/mountinfo/blob/master/src/lib.rs
            let line = line.unwrap();

            let re = Regex::new(r"(\d*)\s(\d*)\s(\d*:\d*)\s([\S]*)\s([\S]*)\s([A-Za-z0-9,]*)\s([A-Za-z0-9:\s]*)\s\- ([\S]*)\s([\S]*)(.*)").unwrap();
            if !re.is_match(&line) {
                panic!("mountinfo parse error, {}", line);
            }

            let caps = re.captures(&line).unwrap();
            let id = String::from(&caps[3]);
            let mount_point = String::from(&caps[5]);

            if info.contains_key(&id) {
                info.get_mut(&id).unwrap().push(mount_point);
            } else {
                info.insert(id, vec![mount_point]);
            }
        }

        return MountInfo { info: info };
    }

    fn get_mount_points_by_id(&self, id: &String) -> Vec<String> {
        let result = self.info.get(id);
        if let Some(result) = result {
            result.clone()
        } else {
            Vec::new()
        }
    }
}

lazy_static! {
    static ref CACAHED_MOUNT_INFO: MountInfo = MountInfo::parse();
}

#[derive(Debug)]
pub struct PartitionDevice {
    pub dev_path: String,

    pub partition_label: Option<String>,
    pub partition_filesystem: String,
    pub partition_size: u64,
    pub usb_model_name: Option<String>,

    pub mounted_points: Vec<String>,
}

impl PartitionDevice {
    fn get_device_properties(device: &Device) -> HashMap<String, String> {
        let mut device_properties: HashMap<String, String> = HashMap::new();
        for entry in device.properties() {
            device_properties.insert(
                String::from(entry.name().to_string_lossy()),
                String::from(entry.value().to_string_lossy()),
            );
        }
        return device_properties;
    }

    fn check_is_usb_device(device: &Device) -> bool {
        return if let Ok(parent) = device.parent_with_subsystem("usb") {
            parent.is_some()
        } else {
            false
        };
    }

    fn from_device(device: Device) -> Option<Self> {
        let device_syspath = String::from(device.syspath().to_string_lossy());
        let device_properties = PartitionDevice::get_device_properties(&device);

        if device_properties.get("ID_FS_TYPE").is_none() {
            return None; // no file system detected, return None
        }

        return if PartitionDevice::check_is_usb_device(&device) {
            // usb partition found
            Some(PartitionDevice {
                dev_path: String::from(
                    device
                        .devnode()
                        .expect(&format!(
                            "DEVNAME for device `{}` get failed",
                            device_syspath
                        ))
                        .to_string_lossy(),
                ),
                partition_label: device_properties
                    .get("ID_FS_LABEL")
                    .map(|s| String::from(s)),
                partition_filesystem: String::from(device_properties.get("ID_FS_TYPE").unwrap()),
                partition_size: device
                    .attribute_value("size")
                    .unwrap()
                    .to_string_lossy()
                    .parse::<u64>()
                    .unwrap()
                    * 512u64,
                usb_model_name: device_properties.get("ID_MODEL").map(|s| String::from(s)),
                mounted_points: CACAHED_MOUNT_INFO.get_mount_points_by_id(&String::from(
                    device
                        .attribute_value("dev")
                        .expect(&format!(
                            "device id for device `{}` get failed",
                            device_syspath
                        ))
                        .to_string_lossy(),
                )),
            })
        } else if device_properties.get("DM_NAME").is_some() {
            // dm partition found, we need do an extra test to determine whether it is a slave of usb device

            let slave_dir = Path::new(&device_syspath).join("slaves");
            let slave_dir_iter = fs::read_dir(&slave_dir).expect(&format!(
                "read_dir of `{}` failed",
                &slave_dir.to_string_lossy()
            ));
            let mut parent_device = None;

            for i in slave_dir_iter {
                if i.is_ok() {
                    parent_device = Some(i.unwrap());
                }
            }

            if parent_device.is_none() {
                return None; // dm device but its parent not found, return None
            }

            let parent_device_syspath =
                String::from(parent_device.unwrap().path().to_string_lossy());
            let parent_device = Device::from_syspath(Path::new(&parent_device_syspath)).unwrap();

            if !PartitionDevice::check_is_usb_device(&parent_device) {
                return None; // dm device's parent not a usb device, return None
            }

            Some(PartitionDevice {
                dev_path: String::from(
                    device
                        .devnode()
                        .expect(&format!(
                            "DEVNAME for device `{}` get failed",
                            device_syspath
                        ))
                        .to_string_lossy(),
                ),
                partition_label: device_properties
                    .get("ID_FS_LABEL")
                    .map(|s| String::from(s)),
                partition_filesystem: String::from(device_properties.get("ID_FS_TYPE").unwrap()),
                partition_size: device
                    .attribute_value("size")
                    .unwrap()
                    .to_string_lossy()
                    .parse::<u64>()
                    .unwrap()
                    * 512u64,
                usb_model_name: PartitionDevice::get_device_properties(&parent_device)
                    .get("ID_MODEL")
                    .map(|s| String::from(s)),
                mounted_points: CACAHED_MOUNT_INFO.get_mount_points_by_id(&String::from(
                    device
                        .attribute_value("dev")
                        .expect(&format!(
                            "device id for device `{}` get failed",
                            device_syspath
                        ))
                        .to_string_lossy(),
                )),
            })
        } else {
            None // not a usb or dm device, return None
        };
    }
}

pub fn get_available_partition_devices() -> Vec<PartitionDevice> {
    let mut partition_devices = Vec::new();

    let mut enumerator = udev::Enumerator::new().unwrap();
    enumerator
        .match_subsystem("block")
        .expect("enumerate device failed");

    for device in enumerator.scan_devices().unwrap() {
        let partition_device = PartitionDevice::from_device(device);
        if let Some(partition_device) = partition_device {
            partition_devices.push(partition_device);
        }
    }

    return partition_devices;
}
