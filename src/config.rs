use prelude::*;
use std::fs::{File};
use std::path::Path;
use serialport::{self,SerialPortType,UsbPortInfo};
use json;

#[derive(Serialize, Deserialize)]
pub struct KeyMap {
    ///The device pin to map this key to.
    pub pin: u8,
    ///The keycodes to map this key to.
    pub keycodes: Vec<u16>,
}

#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DebounceType {
    //Wait for `debounce_ms` from the first keystate change.
    FirstChange,
    //Wait for `debounce_ms` from the last keystate change.
    LastChange,
}

#[derive(Serialize, Deserialize)]
pub struct Config {
    ///What serial port to use to connect to the device.
    pub serial_port: String,
    ///A command-line command to run before proceeding to attempt the connection.
    ///Used to program the device with a suitable server before connecting.
    ///The keyword `{{port}}` will be replaced for the setup port.
    pub previous_command: Option<String>,
    ///How many bits per second to communicate through.
    pub baud_rate: u32,
    ///Keys to map.
    pub key_maps: Vec<KeyMap>,
    ///Milliseconds of debounce.
    pub debounce_ms: f64,
    ///What kind of debounce to use.
    pub debounce_type: DebounceType,
    ///Whether the device should listen to interrupts.
    pub enable_interrupts: bool,
    ///How long to wait for the device to respond.
    pub timeout_ms: u64,
    ///Print all sorts of stuff.
    pub verbose: bool,
}
impl Default for Config {
    fn default() -> Config {
        //Create default config
        Config {
            previous_command: None,
            serial_port: ":auto-usb-arduino".into(),
            baud_rate: 115200,
            key_maps: vec![KeyMap {
                pin: 2,
                keycodes: vec![32],
            }],
            debounce_ms: 1.0,
            debounce_type: DebounceType::LastChange,
            enable_interrupts: false,
            timeout_ms: 3000,
            verbose: false,
        }
    }
}
impl Config {
    ///Load or create a config file.
    ///Never errors, as it will use a default if missing.
    pub fn create<P: AsRef<Path>>(path: P) -> Config {
        //Write configuration file (delayed)
        let write_cfg =
            |cfg: &Config| -> Result<()> { Ok(json::to_writer_pretty(File::create(&path)?, cfg)?) };

        //Read configuration
        let cfg = || -> Result<_> { Ok(json::from_reader(File::open(&path)?)?) };
        match cfg() {
            Ok(cfg) => cfg,
            Err(err) => {
                eprintln!("error reading config file: {}", err);
                eprintln!("using default config");
                let cfg = Config::default();
                if let Err(err) = write_cfg(&cfg) {
                    eprintln!("error writing config file: {}", err);
                }
                cfg
            }
        }
    }
    
    ///Get a physical port name, resolving any wildcards in the config.
    pub fn resolve_port(&self)->Result<String> {
        Ok(match &*self.serial_port {
            portname if portname.starts_with(":auto-usb-") => {
                //Product name substring to look for
                let substr = &portname[":auto-usb-".len()..].to_lowercase();
                //Find within available ports...
                let port = serialport::available_ports()?.into_iter().find(|port| {
                    match port.port_type {
                        //A usb port with a name containing the substring (ignoring case)
                        SerialPortType::UsbPort(UsbPortInfo {
                            product: Some(ref product),
                            ..
                        }) if product.to_lowercase().contains(&*substr) =>
                        {
                            true
                        }
                        _ => false,
                    }
                });
                //Throw an error if no port matched the conditions
                port.ok_or_else(|| {
                    format!(
                        "found no usb serial port containing '{}' in its name",
                        substr
                    )
                })?
                    .port_name
            }
            portname => String::from(portname),
        })
    }
}
