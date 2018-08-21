#ifndef STRUCTS_H
#define STRUCTS_H

const int MAX_KEYS = 128;

const byte MAGIC_NUMBER[8] = {'S','e','r','K','e','y','0','1'};

struct Timer {
  unsigned long timeout;
  bool enabled;

  void init() {
    this->enabled=false;
  }
  
  void set(unsigned long deadline);
  bool check(unsigned long now);
  bool check_now() {return this->check(micros());}
};
//Set the timer to expire on this time.
void Timer::set(unsigned long deadline) {
  this->timeout=deadline;
  this->enabled=true;
}
//Checks whether the timer has expired, handling overflow as best as possible.
bool Timer::check(unsigned long now) {
  bool expired = this->enabled && (signed long)(now-this->timeout)>=0;
  if (expired) {
    this->enabled=false;
  }
  return expired;
}

struct Key {
  //What pin this key corresponds to.
  byte pin;
  //What does the host think about this key's state.
  bool was_down;
  //If any update is to be sent, it must be sent after this timer expires (debounce).
  Timer debounce_timer;

  void init(int pin) {
    this->pin = pin;
    this->was_down = false;
    this->debounce_timer.init();
  }
};

struct State {
  unsigned long debounce_micros;
  bool await_smoothness;
  bool enable_interrupts;
  Key keys[MAX_KEYS];
  int key_count;

  void init() {
    this->debounce_micros=1000;
    this->await_smoothness=true;
    this->enable_interrupts=false;
    this->key_count=0;
  }
};

#endif
