extern crate serialport;
#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate serde_json as json;
extern crate enigo;

use std::path::{Path};
use std::error::{Error};
use std::fs::{File};
use std::io::{Read};
use std::cell::{RefCell};
use std::fmt;
use serialport::{SerialPort,SerialPortSettings,Parity,DataBits,StopBits,FlowControl};
use enigo::{Enigo,KeyboardControllable};

type Result<T> = ::std::result::Result<T,Box<Error>>;
#[derive(Debug)]
struct BoxErrorMsg {
  msg: String,
  cause: Box<Error>,
}
impl fmt::Display for BoxErrorMsg {
  fn fmt(&self,f: &mut fmt::Formatter)->fmt::Result {
    writeln!(f,"{}",self.msg)?;
    write!(f," caused by: {}",self.cause)
  }
}
impl Error for BoxErrorMsg {
  fn cause(&self)->Option<&Error> {Some(&*self.cause)}
}
trait ResultBoxExt {
  type Mapped;
  fn chain<M: Into<String>>(self,msg: M)->Self::Mapped;
}
impl<T> ResultBoxExt for ::std::result::Result<T,Box<Error>> {
  type Mapped = ::std::result::Result<T,BoxErrorMsg>;
  fn chain<M: Into<String>>(self,msg: M)->Self::Mapped {
    self.map_err(|err| BoxErrorMsg{msg: msg.into(),cause: err})
  }
}
trait ResultExt {
  type Mapped;
  fn chain<M: Into<String>>(self,msg: M)->Self::Mapped;
}
impl<T,E: Error+'static> ResultExt for ::std::result::Result<T,E> {
  type Mapped = ::std::result::Result<T,BoxErrorMsg>;
  fn chain<M: Into<String>>(self,msg: M)->Self::Mapped {
    self.map_err(|err| BoxErrorMsg{msg: msg.into(),cause: Box::new(err)})
  }
}

#[derive(Serialize,Deserialize)]
struct Config {
  pub serial_port: String,
  pub baud_rate: u32,
  pub mapping: Vec<u16>,
}
impl Default for Config {
  fn default()->Config {
    //Create default config
    Config{
      serial_port: serialport::available_ports().filter(|port| port.port_type()),
      baud_rate: 115200,
      mapping: Vec::new(),
    }
  }
}
impl Config {
  ///Load or create a config file.
  ///Never errors, as it will use a default if missing.
  pub fn create<P: AsRef<Path>>(path: P)->Config {
    let write_cfg=|cfg: &Config|->Result<()> {Ok(json::to_writer(File::create(&path)?,cfg)?)};
    
    let cfg=||->Result<_> {Ok(json::from_reader(File::open(&path)?)?)};
    match cfg() {
      Ok(cfg)=>cfg,
      Err(err)=>{
        eprintln!("error reading config file: {}",err);
        eprintln!("using default config");
        let cfg=Config::default();
        if let Err(err) = write_cfg(&cfg) {
          eprintln!("error writing config file: {}",err);
        }
        cfg
      },
    }
  }
}

struct Connection {
  serial: Box<SerialPort>,
}
impl Connection {
  pub fn open(cfg: &Config)->Result<Connection> {
    let serial=serialport::open_with_settings(&cfg.serial_port,&SerialPortSettings{
      baud_rate: cfg.baud_rate,
      ..Default::default()
    })?;
    let mut conn=Connection{
      serial,
    };
    conn.initialize(cfg)?;
    Ok(conn)
  }
  
  ///Read the magic number, recognizing and opening the connection.
  fn initialize(&mut self,cfg: &Config)->Result<()> {
    //Check magic number
    let mut magic_buf=[0; 8];
    self.serial.read_exact(&mut magic_buf)?;
    if &magic_buf!=b"SERKEYv1" {
      return Err("device is not a valid v1 serial keyboard".into());
    }
    //Check key count
    let mut key_count=[0];
    self.serial.read_exact(&mut key_count)?;
    let key_count=key_count[0] as usize;
    if key_count>cfg.mapping.len() {
      println!("device has {} unmapped available keys",key_count-cfg.mapping.len());
    }else if key_count<cfg.mapping.len() {
      println!("there are {} excess key mappings",cfg.mapping.len()-key_count);
    }
    //All ok
    Ok(())
  }
  
  ///Block until an event is read.
  fn read_event(&mut self)->Result<Event> {
    let mut event=[0; 2];
    self.serial.read_exact(&mut event)?;
    Ok(Event::key_update(event[0],event[1]))
  }
}

enum Event {
  KeyDown(u8),
  KeyUp(u8),
}
impl Event {
  fn key_update(key_idx: u8,state: u8)->Event {
    if state!=0 {
      Event::KeyDown(key_idx)
    }else{
      Event::KeyUp(key_idx)
    }
  }
  
  fn consume(self,cfg: &Config)->Result<()> {
    //Static enigo instance
    thread_local! {
      static ENIGO: RefCell<Enigo> = RefCell::new(Enigo::new());
    }
    
    //Key state change helper
    fn key_change(cfg: &Config,idx: u8,func: fn(&mut Enigo,enigo::Key)) {
      cfg.mapping.get(idx as usize).and_then(|keycode| {
        ENIGO.with(|enigo| func(&mut *enigo.borrow_mut(),enigo::Key::Raw(*keycode)));
        Some(())
      });
    }
    
    //Check event type and act accordingly
    match self {
      Event::KeyDown(idx)=>{
        key_change(cfg,idx,Enigo::key_down);
      },
      Event::KeyUp(idx)=>{
        key_change(cfg,idx,Enigo::key_up);
      }
    }
    
    Ok(())
  }
}

pub fn run()->Result<()> {
  //Read configuration files
  let config=Config::create("config.txt");
  
  //Open and handle connection
  let mut conn=Connection::open(&config).chain("failed to open connection")?;
  loop {
    conn.read_event()?.consume(&config)?;
  }
}
