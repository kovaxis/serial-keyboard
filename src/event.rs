use prelude::*;
use config::{Config};
use enigo::{self,Enigo,KeyboardControllable};
use std::cell::RefCell;

pub enum Event {
    KeyDown(u8),
    KeyUp(u8),
}
impl Event {
    pub fn from_raw(ev_byte: u8) -> Event {
        let idx = ev_byte & 0x7F;
        let state = (ev_byte & 0x80) != 0;
        if state {
            Event::KeyDown(idx)
        } else {
            Event::KeyUp(idx)
        }
    }

    pub fn consume(self, cfg: &Config) -> Result<()> {
        //Static enigo instance
        thread_local! {
          static ENIGO: RefCell<Enigo> = RefCell::new(Enigo::new());
        }

        //Key state change helper
        fn key_change<F: FnMut(&mut Enigo,enigo::Key)>(cfg: &Config, idx: u8, mut func: F) {
            cfg.key_maps.get(idx as usize).and_then(|keymap| {
                ENIGO.with(|enigo| for keycode in keymap.keycodes.iter() {
                    println!("updating physical keycode {}",keycode);
                    func(&mut *enigo.borrow_mut(), enigo::Key::Raw(*keycode))
                });
                Some(())
            });
        }

        //Check event type and act accordingly
        match self {
            Event::KeyDown(idx) => {
                println!("pressing virtual key {}",idx);
                key_change(cfg, idx, Enigo::key_down);
            }
            Event::KeyUp(idx) => {
                println!("releasing virtual key {}",idx);
                key_change(cfg, idx, Enigo::key_up);
            }
        }

        Ok(())
    }
}
