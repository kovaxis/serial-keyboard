extern crate serialport;
#[macro_use]
extern crate serde_derive;
extern crate enigo;
extern crate serde;
extern crate serde_json as json;
extern crate subprocess;

use prelude::*;
use subprocess::{Exec};
use std::thread;
use std::time::{Duration};

use config::{Config};
use connection::{Connection};

mod config;
mod connection;
mod event;

mod prelude {
    use std::error::Error;
    
    pub use std::{fmt};
    pub use std::result::Result as StdResult;
    
    pub type Result<T> = ::std::result::Result<T, Box<Error>>;
    #[derive(Debug)]
    pub struct BoxErrorMsg {
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
    pub trait ResultBoxExt {
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
    pub trait ResultExt {
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
}

pub fn run() -> Result<()> {
    //Read configuration files
    let config = Config::create("config.txt");
    if config.verbose {
        println!("being verbose");
    }

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
        thread::sleep(Duration::from_millis(2000));
    }

    //Open and handle connection
    let mut conn = Connection::open(&config).chain("failed to open connection")?;
    println!("handling device events");
    loop {
        conn.read_event(&config)
            .chain("failed to read event from device")?
            .consume(&config)
            .chain("failed to execute device event")?;
    }
}

///Called whether the main function fails or suceeds.
pub fn finish_off() {
    Exec::shell("pause").join().ok();
}
