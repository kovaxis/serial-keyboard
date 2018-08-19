extern crate serialport;
#[macro_use]
extern crate serde_derive;
extern crate enigo;
extern crate serde;
extern crate serde_json as json;
extern crate subprocess;

use enigo::{Enigo, KeyboardControllable};
use serialport::{SerialPort, SerialPortSettings, SerialPortType, UsbPortInfo};
use subprocess::{Exec};
use std::cell::RefCell;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

const MAGIC_NUMBER: &[u8] = b"SerKey01";

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
enum SetupCommand {
    Finish,
    AddKey,
    SetDebounce,
    AwaitSmoothness,
    Reset,
    EnableInterrupts,
}
impl SetupCommand {
    fn code(self) -> u8 {
        use self::SetupCommand::*;
        match self {
            Finish => 0x0F,
            AddKey => 0xAD,
            SetDebounce => 0xDB,
            AwaitSmoothness => 0xAE,
            Reset => 0xEE,
            EnableInterrupts => 0xEA,
        }
    }
}

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
struct KeyMap {
    ///The device pin to map this key to.
    pub pin: u8,
    ///The keycodes to map this key to.
    pub keycodes: Vec<u16>,
}

#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum DebounceType {
    //Wait for `debounce_ms` from the first keystate change.
    FirstChange,
    //Wait for `debounce_ms` from the last keystate change.
    LastChange,
}

#[derive(Serialize, Deserialize)]
struct Config {
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
        let portname = cfg.resolve_port()?;

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

    fn read_magic(&mut self, _cfg: &Config) -> Result<()> {
        let mut magic_idx = 0;
        let mut garbage = 0;
        while magic_idx < MAGIC_NUMBER.len() {
            let mut byte = [0; 1];
            self.serial
                .read(&mut byte)
                .chain("reading magic number failed")?;
            let byte = byte[0];
            print!("{}", byte as char);
            if byte == MAGIC_NUMBER[magic_idx] {
                magic_idx += 1;
            } else {
                garbage += magic_idx + 1;
                magic_idx = 0;
            }
        }
        println!();
        println!("received magic number after {} bytes of garbage", garbage);
        Ok(())
    }

    ///Read the magic number, recognizing and opening the connection.
    fn initialize(&mut self, cfg: &Config) -> Result<()> {
        //Send a reboot message in case the client is already running
        self.serial
            .write_all(&[SetupCommand::Reset.code(), 0, 0])
            .chain("failed to write reset command")?;

        //Send magic number
        self.serial
            .write_all(MAGIC_NUMBER)
            .chain("failed to send magic number")?;

        //Receive magic number
        self.read_magic(cfg)?;
        self.serial.set_timeout(Duration::from_millis(0))?;

        //Set debounce length
        let debounce = (cfg.debounce_ms * 1000.0)
            .min(u32::max_value() as f64)
            .max(0.0) as u32;
        self.serial.write_all(&[
            SetupCommand::SetDebounce.code(),
            0,
            4,
            ((debounce >> 24) & 0xFF) as u8,
            ((debounce >> 16) & 0xFF) as u8,
            ((debounce >> 8) & 0xFF) as u8,
            ((debounce >> 0) & 0xFF) as u8,
        ])?;
        //Set debounce type
        match cfg.debounce_type {
            DebounceType::FirstChange => {
                self.serial
                    .write_all(&[SetupCommand::AwaitSmoothness.code(), 0, 1, 0])?;
            }
            DebounceType::LastChange => {
                self.serial
                    .write_all(&[SetupCommand::AwaitSmoothness.code(), 0, 1, 1])?;
            }
        }
        //Setup keys
        for keymap in cfg.key_maps.iter() {
            self.serial
                .write_all(&[SetupCommand::AddKey.code(), 0, 1, keymap.pin])
                .chain("failed to setup key with device")?;
        }
        //Enable or disable interrupts
        self.serial.write_all(&[
            SetupCommand::EnableInterrupts.code(),
            0,
            1,
            if cfg.enable_interrupts { 1 } else { 0 },
        ])?;
        //Send setup finish
        self.serial
            .write_all(&[SetupCommand::Finish.code(), 0, 0])
            .chain("failed to finish setup")?;
        
        //Read setup output (until an empty line)
        println!("device setup output:");
        let mut line_buf = Vec::new();
        loop {
            line_buf.clear();
            //Read all bytes until a newline
            loop {
                let mut char_buf = [0; 1];
                self.serial
                    .read_exact(&mut char_buf)
                    .chain("failed to read setup log")?;
                if &char_buf == b"\n" {
                    break;
                } else {
                    line_buf.push(char_buf[0]);
                }
            }
            //Quit if an empty line, otherwise print
            let line = String::from_utf8_lossy(&line_buf);
            let line = line.trim();
            if line.is_empty() {
                break;
            } else {
                println!(" {}", line);
            }
        }
        println!("--- setup finished ---");

        //Set an infinite timeout
        self.serial.set_timeout(Duration::from_millis(0))?;
        //All ok
        Ok(())
    }

    ///Block until an event is read.
    fn read_event(&mut self) -> Result<Event> {
        let mut event = [0; 1];
        self.serial.read_exact(&mut event)?;
        Ok(Event::from_raw(event[0]))
    }
}

enum Event {
    KeyDown(u8),
    KeyUp(u8),
}
impl Event {
    fn from_raw(ev_byte: u8) -> Event {
        let idx = ev_byte & 0x7F;
        let state = (ev_byte & 0x80) != 0;
        if state {
            Event::KeyDown(idx)
        } else {
            Event::KeyUp(idx)
        }
    }

    fn consume(self, cfg: &Config) -> Result<()> {
        //Static enigo instance
        thread_local! {
          static ENIGO: RefCell<Enigo> = RefCell::new(Enigo::new());
        }

        //Key state change helper
        fn key_change(cfg: &Config, idx: u8, func: fn(&mut Enigo, enigo::Key)) {
            cfg.key_maps.get(idx as usize).and_then(|keymap| {
                ENIGO.with(|enigo| for keycode in keymap.keycodes.iter() {
                    func(&mut *enigo.borrow_mut(), enigo::Key::Raw(*keycode))
                });
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

    //Run previous command if setup
    if let Some(ref cmd) = config.previous_command {
        let cmd=cmd.replace("{{port}}",&config.resolve_port().unwrap_or_else(|_| config.serial_port.clone()));
        println!("running setup previous command: {}",cmd);
        match Exec::shell(&cmd).join() {
            Ok(ref status) if status.success() => {
                println!("successfully ran previous command");
            },
            Ok(status) => {
                eprintln!("error running previous command, exit status {:?}",status);
            },
            Err(err) => {
                eprintln!("failed to run previous command: {}",err);
            },
        }
        println!();
    }

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
