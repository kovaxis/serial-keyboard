#ifndef STRUCTS_H
#define STRUCTS_H

const int MAX_KEYS = 128;

const byte MAGIC_NUMBER[8] = {'S','e','r','K','e','y','0','1'};

struct Key {
  //What pin this key corresponds to.
  int pin;
  //If any update is to be sent, it must be sent after this time (debounce).
  unsigned long next_update;
  //What does the host think about this key's state.
  bool was_down;
};
void Key_init(Key* key,int pin) {
  key->pin = pin;
  key->next_update = 0;
  key->was_down = false;
}

struct State {
  unsigned long debounce_micros;
  bool await_smoothness;
  bool enable_interrupts;
  Key keys[MAX_KEYS];
  int key_count;
};

void State_init(State* state) {
  state->debounce_micros=1000;
  state->await_smoothness=true;
  state->enable_interrupts=false;
  state->key_count=0;
}

#endif
