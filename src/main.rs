extern crate serial_keyboard;

fn main() {
  if let Err(err) = serial_keyboard::run() {
    eprintln!("fatal error: {}",err);
  }
  ::std::process::Command::new("pause").output().ok();
}
