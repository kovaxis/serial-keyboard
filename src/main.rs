extern crate serial_keyboard;

fn main() {
  if let Err(err) = serial_keyboard::run() {
    eprintln!("fatal error: {}",err);
  }
  serial_keyboard::finish_off();
}
