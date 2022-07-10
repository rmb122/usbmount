use byte_unit::Byte;
use dialoguer::{theme::ColorfulTheme, Select};
use std::{collections::HashMap, fs, path::Path};
use sys_mount::{unmount, Mount, MountFlags, SupportedFilesystems, UnmountFlags};

use usbmount::{get_available_partition_devices, PartitionDevice};
use clap::{Parser, Subcommand};
use console::Term;

#[derive(Parser)]
#[clap(version = "1.0")]
#[clap(about = "External storage devices mounting tool", long_about = None)]
struct ArgumentParser {
    #[clap(short, long, parse(from_flag))]
    skip_escalate: bool,

    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[clap(visible_alias = "m")]
    Mount {
        #[clap(short, long, default_value_t = String::from("/var/run/media/"))]
        auto_mount_dir: String,

        #[clap(short, long)]
        mount_option: Option<String>,

        #[clap(action)]
        dev_path: Option<String>,

        #[clap(action)]
        mount_path: Option<String>,
    },

    #[clap(visible_alias = "u")]
    Umount {
        #[clap(action)]
        dev_path: Option<String>,
    },

    #[clap(visible_alias = "i")]
    Info {},
}

static IDENTIFY_FILE: &str = ".create_by_usbmount";
static MOUNT_WITH_DEFAULT_OPTION_FILESYSTEM: [&str; 3] = ["ntfs", "vfat", "exfat"];

extern "system" {
    fn geteuid() -> u32;
    fn getegid() -> u32;
}

fn safe_geteuid() -> u32 {
    unsafe { geteuid() }
}

fn safe_getegid() -> u32 {
    unsafe { getegid() }
}

fn format_optional_string(str: &Option<String>) -> String {
    if let Some(str) = str {
        return format!("\"{}\"", str);
    } else {
        return String::from("None");
    }
}

fn format_partition_size(size: u64) -> String {
    let result = Byte::from_bytes(size.into());
    return format!("\"{}\"", result.get_appropriate_unit(true).to_string());
}

fn format_mount_points(mount_points: &Vec<String>) -> String {
    let mut ret = String::from("[");
    mount_points
        .iter()
        .for_each(|x| ret.push_str(&format!("\"{}\",", x)));
    if ret.ends_with(',') {
        ret.remove(ret.len() - 1);
    }
    ret.push(']');
    return ret;
}

fn select_mount_device(mut devices_map: HashMap<String, PartitionDevice>) -> PartitionDevice {
    let mut device_vec: Vec<&PartitionDevice> = devices_map
        .values()
        .filter(|d| d.mounted_points.len() == 0)
        .collect();
    let selections: Vec<String> = device_vec
        .iter()
        .map(|device| {
            format!(
                "{} [FileSystem({}) Size({}) Label({}) Model({})]",
                device.dev_path,
                format_optional_string(&Some(device.partition_filesystem.clone())),
                format_partition_size(device.partition_size),
                format_optional_string(&device.partition_label),
                format_optional_string(&device.usb_model_name),
            )
        })
        .collect();

    if selections.len() == 0 {
        eprintln!("all device already mounted");
        std::process::exit(-1);
    }

    let select_index = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("pick the device you want mount")
        .default(0)
        .items(&selections)
        .report(false)
        .interact_opt();

    if let Ok(Some(select_index)) = select_index {
        let dev_path = device_vec.remove(select_index).dev_path.clone();
        devices_map.remove(&dev_path).unwrap()
    } else {
        // user exit, recover console messup by dialoguer
        Term::stderr().show_cursor().unwrap();
        std::process::exit(-1);
    }
}

fn select_umount_device(mut devices_map: HashMap<String, PartitionDevice>) -> PartitionDevice {
    let mut device_vec: Vec<&PartitionDevice> = devices_map
        .values()
        .filter(|d| d.mounted_points.len() > 0)
        .collect();
    let selections: Vec<String> = device_vec
        .iter()
        .map(|device| {
            format!(
                "{} [MountPoint({}) FileSystem({}) Size({}) Label({}) Model({})]",
                device.dev_path,
                format_mount_points(&device.mounted_points),
                format_optional_string(&Some(device.partition_filesystem.clone())),
                format_partition_size(device.partition_size),
                format_optional_string(&device.partition_label),
                format_optional_string(&device.usb_model_name),
            )
        })
        .collect();

    if selections.len() == 0 {
        eprintln!("no mounted device found");
        std::process::exit(-1);
    }

    let select_index = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("pick the device you want umount")
        .default(0)
        .items(&selections)
        .report(false)
        .interact_opt();

    if let Ok(Some(select_index)) = select_index {
        let dev_path = device_vec.remove(select_index).dev_path.clone();
        devices_map.remove(&dev_path).unwrap()
    } else {
        // user exit, recover console messup by dialoguer
        Term::stderr().show_cursor().unwrap();
        std::process::exit(-1);
    }
}

fn main() {
    ctrlc::set_handler(|| {}).unwrap(); // ignore ctrl-c, let the process exit after sudo exit

    let argument_parser = ArgumentParser::parse();
    match argument_parser.command {
        Commands::Mount{..} | Commands::Umount{..} => {
            if !argument_parser.skip_escalate && safe_geteuid() != 0 {
                sudo::escalate_if_needed().expect("escalate error");
            }
        }
        _ => {}
    }

    let mut devices_map: HashMap<String, PartitionDevice> = get_available_partition_devices()
        .into_iter()
        .map(|device| (device.dev_path.clone(), device))
        .collect();

    match &argument_parser.command {
        Commands::Mount {
            auto_mount_dir,
            mount_option,
            dev_path,
            mount_path,
        } => {
            let device = match (dev_path, devices_map.len()) {
                (None, 1) => {
                    // dev_path not provided but only one block device, use it as default
                    let first_key: String = devices_map.keys().next().unwrap().clone();
                    devices_map.remove(&first_key).unwrap()
                }
                (None, 0) => {
                    // dev_path not provided and block device no found, exit
                    eprintln!("usb block device not found");
                    std::process::exit(-1);
                }
                (None, devices_count) if devices_count > 1 => {
                    // dev_path not provided and multiple block device, choose one to mount
                    select_mount_device(devices_map)
                }
                (Some(dev_path), _) => {
                    // dev_path provided
                    if !devices_map.contains_key(dev_path) {
                        eprintln!(
                            "device `{}` not exist or its not a portable block device",
                            dev_path
                        );
                        std::process::exit(-1);
                    }
                    devices_map.remove(dev_path).unwrap()
                }
                (_, _) => {
                    panic!("unexpected status")
                }
            };

            if device.mounted_points.len() > 0 {
                eprintln!(
                    "device `{}` alreday mounted at `{}`",
                    device.dev_path, device.mounted_points[0]
                );
                std::process::exit(-1);
            } else {
                let mount_path = if let Some(mount_path) = mount_path {
                    String::from(mount_path)
                } else {
                    let base_dir = if device.partition_label.is_some() {
                        device.partition_label.unwrap()
                    } else {
                        String::from(
                            Path::new(&device.dev_path)
                                .file_name()
                                .unwrap()
                                .to_string_lossy(),
                        )
                    };

                    let username = std::env::var("SUDO_USER").unwrap_or(whoami::username());

                    let mut mount_path = Path::new(&auto_mount_dir).join(username).join(base_dir);
                    if mount_path.exists() {
                        let mut deduplicate_id = 0;
                        loop {
                            let new_mount_path =
                                format!("{}-{}", mount_path.to_str().unwrap(), deduplicate_id);
                            let new_mount_path = Path::new(&new_mount_path);

                            if new_mount_path.exists() {
                                deduplicate_id = deduplicate_id + 1;
                            } else {
                                mount_path = new_mount_path.to_path_buf();
                                break;
                            }
                        }
                    }
                    fs::create_dir_all(mount_path.clone()).expect(&format!(
                        "mount path `{}` create error",
                        mount_path.to_string_lossy()
                    ));

                    // create a speacil file so we can identify the dirctory we created
                    fs::File::create(mount_path.join(IDENTIFY_FILE)).expect(&format!(
                        "create identify file `{}` error",
                        mount_path.join(IDENTIFY_FILE).to_string_lossy()
                    ));

                    String::from(mount_path.to_string_lossy())
                };

                let mount_option = if let Some(mount_option) = mount_option {
                    String::from(mount_option)
                } else {
                    if MOUNT_WITH_DEFAULT_OPTION_FILESYSTEM
                        .contains(&device.partition_filesystem.as_str())
                    {
                        format!(
                            "uid={},gid={}",
                            std::env::var("SUDO_UID").unwrap_or(safe_geteuid().to_string()),
                            std::env::var("SUDO_GID").unwrap_or(safe_getegid().to_string())
                        )
                    } else {
                        String::new()
                    }
                };

                // mount device
                match Mount::new(
                    &device.dev_path,
                    &mount_path,
                    &SupportedFilesystems::new().unwrap(),
                    MountFlags::empty(),
                    Some(&mount_option),
                ) {
                    Ok(_) => {
                        println!("{}", mount_path);
                    }
                    Err(why) => {
                        eprintln!(
                            "failed to mount {} to {}: {}",
                            device.dev_path, mount_path, why
                        );
                        std::process::exit(-1);
                    }
                }
            }
        }
        Commands::Umount { dev_path } => {
            let device = match (dev_path, devices_map.len()) {
                (None, 1) => {
                    // dev_path not provided but only one block device, use it as default
                    let first_key: String = devices_map.keys().next().unwrap().clone();
                    devices_map.remove(&first_key).unwrap()
                }
                (None, 0) => {
                    // dev_path not provided and block device no found, exit
                    eprintln!("usb block device not found");
                    std::process::exit(-1);
                }
                (None, device_count) if device_count > 1 => {
                    // dev_path not provided and multiple block device, choose one to umount
                    select_umount_device(devices_map)
                }
                (Some(dev_path), _) => {
                    // dev_path provided
                    if !devices_map.contains_key(dev_path) {
                        eprintln!(
                            "device `{}` not exist or its not a portable block device",
                            dev_path
                        );
                        std::process::exit(-1);
                    }
                    devices_map.remove(dev_path).unwrap()
                }
                (_, _) => {
                    panic!("unexpected status")
                }
            };

            if device.mounted_points.len() == 0 {
                eprintln!("device `{}` don't have any mount point", device.dev_path);
                std::process::exit(-1);
            } else {
                for mount_point in device.mounted_points {
                    let result = unmount(&mount_point, UnmountFlags::empty());
                    if result.is_err() {
                        eprintln!(
                            "when umount device `{}` mount point `{}`, encount error `{}`",
                            device.dev_path,
                            &mount_point,
                            result.err().unwrap()
                        );
                    } else {
                        println!("{}", mount_point);
                        let identify_file_path = Path::new(&mount_point).join(IDENTIFY_FILE);
                        if identify_file_path.exists() {
                            fs::remove_file(&identify_file_path).expect(&format!(
                                "remove identify file `{}` error",
                                identify_file_path.to_string_lossy()
                            ));
                            fs::remove_dir(&mount_point).expect(&format!(
                                "remove mount point directory `{}` error",
                                mount_point
                            ));
                        }
                    }
                }
            }
        }
        Commands::Info {} => {
            devices_map.values().for_each(|device| {
                println!(
                    "{} [MountPoint({}) FileSystem({}) Size({}) Label({}) Model({})]",
                    device.dev_path,
                    format_mount_points(&device.mounted_points),
                    format_optional_string(&Some(device.partition_filesystem.clone())),
                    format_partition_size(device.partition_size),
                    format_optional_string(&device.partition_label),
                    format_optional_string(&device.usb_model_name),
                );
            });
        }
    }
}
