use serialport::{self,SerialPort,SerialPortType,SerialPortSettings};
use prelude::*;
use config::{Config,DebounceType};
use event::{Event};
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

pub struct Connection {
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
                timeout: Duration::from_millis(cfg.timeout_ms),
                ..Default::default()
            },
        ).chain("failed to open serial port, ensure device is connected and the correct port is being used")?;

        //Create and init connection
        let mut conn = Connection { serial };
        conn.initialize(cfg)
            .chain("failed to initialize connection")?;
        Ok(conn)
    }

    fn read_magic(&mut self, cfg: &Config) -> Result<()> {
        let mut magic_idx = 0;
        let mut garbage = 0;
        if cfg.verbose {
            print!("reading magic number: '");
        }
        while magic_idx < MAGIC_NUMBER.len() {
            let mut byte = [0; 1];
            self.serial
                .read(&mut byte)
                .chain("reading magic number failed")?;
            let byte = byte[0];
            if cfg.verbose {
                print!("{}", byte as char);
            }
            if byte == MAGIC_NUMBER[magic_idx] {
                magic_idx += 1;
            } else {
                garbage += magic_idx + 1;
                magic_idx = 0;
            }
        }
        if cfg.verbose {
            println!("'");
        }
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
    pub fn read_event(&mut self,cfg: &Config) -> Result<Event> {
        let mut event = [0; 1];
        self.serial.read_exact(&mut event)?;
        if cfg.verbose {
            println!("received event byte 0x{:X}",event[0]);
        }
        Ok(Event::from_raw(event[0]))
    }
}
