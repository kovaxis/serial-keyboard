extern crate serialport;
#[macro_use]
extern crate serde_derive;
extern crate enigo;
extern crate serde;
extern crate serde_json as json;

use enigo::{Enigo, KeyboardControllable};
use serialport::{SerialPort, SerialPortSettings, SerialPortType, UsbPortInfo};
use std::cell::RefCell;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

const MAGIC_NUMBER: &[u8] = b"SerKey01";

type Result<T> = ::std::result::Result<T, Box<Error>>;
#[derive(Debug)]
struct BoxErrorMsg {
    msg: String,
    cause: Box<Error>,
}
impl fmt::Display for BoxErrorMsg {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "{}", self.msg)?;
        write!(f, " caused by: {}", self.cause)
    }
}
impl Error for BoxErrorMsg {
    fn cause(&self) -> Option<&Error> {
        Some(&*self.cause)
    }
}
trait ResultBoxExt {
    type Mapped;
    fn chain<M: Into<String>>(self, msg: M) -> Self::Mapped;
}
impl<T> ResultBoxExt for ::std::result::Result<T, Box<Error>> {
    type Mapped = ::std::result::Result<T, BoxErrorMsg>;
    fn chain<M: Into<String>>(self, msg: M) -> Self::Mapped {
        self.map_err(|err| BoxErrorMsg {
            msg: msg.into(),
            cause: err,
        })
    }
}
trait ResultExt {
    type Mapped;
    fn chain<M: Into<String>>(self, msg: M) -> Self::Mapped;
}
impl<T, E: Error + 'static> ResultExt for ::std::result::Result<T, E> {
    type Mapped = ::std::result::Result<T, BoxErrorMsg>;
    fn chain<M: Into<String>>(self, msg: M) -> Self::Mapped {
        self.map_err(|err| BoxErrorMsg {
            msg: msg.into(),
            cause: Box::new(err),
        })
    }
}

#[derive(Serialize, Deserialize)]
struct Config {
    pub serial_port: String,
    pub baud_rate: u32,
    pub mapping: Vec<u16>,
}
impl Default for Config {
    fn default() -> Config {
        //Create default config
        Config {
            serial_port: ":auto-usb-arduino".into(),
            baud_rate: 115200,
            mapping: Vec::new(),
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
}

struct Connection {
    serial: Box<SerialPort>,
}
impl Connection {
    pub fn open(cfg: &Config) -> Result<Connection> {
        //Print available ports
        println!("available ports:");
        for port in serialport::available_ports().chain("failed to enumerate available ports")? {
            print!(" {}: ", port.port_name);
            match port.port_type {
                SerialPortType::UsbPort(info) => {
                    println!("usb port");
                    println!("  vendor id: 0x{:X}", info.vid);
                    println!("  product id: 0x{:X}", info.pid);
                    println!(
                        "  serial number: '{}'",
                        info.serial_number.unwrap_or("unavailable".into())
                    );
                    println!(
                        "  manufacturer: '{}'",
                        info.manufacturer.unwrap_or("unavailable".into())
                    );
                    println!(
                        "  product name: '{}'",
                        info.product.unwrap_or("unavailable".into())
                    );
                }
                SerialPortType::PciPort => println!("pci port"),
                SerialPortType::BluetoothPort => println!("bluetooth port"),
                SerialPortType::Unknown => println!("unknown port type"),
            }
        }

        //Get serial port name
        let portname = match &*cfg.serial_port {
            portname if portname.starts_with(":auto-usb-") => {
                //Product name substring to look for
                let substr = &portname[":auto-usb-".len()..].to_lowercase();
                //Find within available ports...
                let port = serialport::available_ports()?
                    .into_iter()
                    .find(|port| match port.port_type {
                        //A usb port with a name containing the substring (ignoring case)
                        SerialPortType::UsbPort(UsbPortInfo {
                            product: Some(ref product),
                            ..
                        })
                            if product.to_lowercase().contains(&*substr) =>
                        {
                            true
                        }
                        _ => false,
                    });
                //Throw an error if no port matched the conditions
                port.ok_or_else(|| {
                    format!(
                        "found no usb serial port containing '{}' in its name",
                        substr
                    )
                })?.port_name
            }
            portname => String::from(portname),
        };

        //Open port
        println!("opening serial port '{}'", portname);
        let serial = serialport::open_with_settings(
            &portname,
            &SerialPortSettings {
                baud_rate: cfg.baud_rate,
                timeout: Duration::from_millis(1000),
                ..Default::default()
            },
        ).chain("failed to open serial port, ensure device is connected and the correct port is being used")?;

        //Create and init connection
        let mut conn = Connection { serial };
        conn.initialize(cfg)
            .chain("failed to initialize connection")?;
        Ok(conn)
    }

    ///Read the magic number, recognizing and opening the connection.
    fn initialize(&mut self, cfg: &Config) -> Result<()> {
        //Check magic number
        let mut magic_buf = [0; 8];
        self.serial
            .read_exact(&mut magic_buf)
            .chain("failed to read magic number")?;
        if &magic_buf != MAGIC_NUMBER {
            return Err(format!(
                "magic number mismatch: not a valid {} connection",
                ::std::str::from_utf8(MAGIC_NUMBER).unwrap()
            ).into());
        }
        println!("magic number matches");
        //Check key count
        let mut key_count = [0];
        self.serial
            .read_exact(&mut key_count)
            .chain("failed to read keycount")?;
        let key_count = key_count[0] as usize;
        if key_count > cfg.mapping.len() {
            println!(
                "device has {} unmapped available keys",
                key_count - cfg.mapping.len()
            );
        } else if key_count < cfg.mapping.len() {
            println!(
                "there are {} excess key mappings",
                cfg.mapping.len() - key_count
            );
        }
        println!("mapping {} keys", usize::min(key_count, cfg.mapping.len()));
        //Set an infinite timeout
        self.serial.set_timeout(Duration::from_millis(0))?;
        //All ok
        Ok(())
    }

    ///Block until an event is read.
    fn read_event(&mut self) -> Result<Event> {
        let mut event = [0; 2];
        self.serial.read_exact(&mut event)?;
        Ok(Event::key_update(event[0], event[1]))
    }
}

enum Event {
    KeyDown(u8),
    KeyUp(u8),
}
impl Event {
    fn key_update(key_idx: u8, state: u8) -> Event {
        if state != 0 {
            if state != 1 {
                println!("nonstandard state {} (expected 0 or 1)", state)
            }
            Event::KeyDown(key_idx)
        } else {
            Event::KeyUp(key_idx)
        }
    }

    fn consume(self, cfg: &Config) -> Result<()> {
        //Static enigo instance
        thread_local! {
          static ENIGO: RefCell<Enigo> = RefCell::new(Enigo::new());
        }

        //Key state change helper
        fn key_change(cfg: &Config, idx: u8, func: fn(&mut Enigo, enigo::Key)) {
            cfg.mapping.get(idx as usize).and_then(|keycode| {
                ENIGO.with(|enigo| func(&mut *enigo.borrow_mut(), enigo::Key::Raw(*keycode)));
                Some(())
            });
        }

        //Check event type and act accordingly
        match self {
            Event::KeyDown(idx) => {
                key_change(cfg, idx, Enigo::key_down);
            }
            Event::KeyUp(idx) => {
                key_change(cfg, idx, Enigo::key_up);
            }
        }

        Ok(())
    }
}

pub fn run() -> Result<()> {
    //Read configuration files
    let config = Config::create("config.txt");

    //Open and handle connection
    let mut conn = Connection::open(&config).chain("failed to open connection")?;
    println!("handling device events");
    loop {
        conn.read_event()
            .chain("failed to read event from device")?
            .consume(&config)
            .chain("failed to execute device event")?;
    }
}
