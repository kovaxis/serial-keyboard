#include "structs.h"

//Finish parsing setup commands and listen for keypresses.
const byte SETUPCMD_FINISH=0x0F;
//Register a key along with its associated pin.
const byte SETUPCMD_ADD_KEY=0xAD;
//Set the debounce timer length in microseconds.
//The debounce timer prevents the device from sending updates within a small timespan from the last (or first, see 'AWAIT_SMOOTHNESS') state change.
//Keystate changes will be queued and sent as soon as the timer expires.
const byte SETUPCMD_SET_DEBOUNCE=0xDB;
//Whether keystate changes reset the debounce timer.
//If enabled, debounce will count from the _last_ keystate change.
//If disabled, debounce will count from the _first_ keystate change.
const byte SETUPCMD_AWAIT_SMOOTHNESS=0xAE;
//Reset the device, or at least the program.
//Can be sent as a single byte when handling input.
const byte CMD_RESET=0xEE;
//Enable or disable pinstate change interrupts.
//Usually unnecessary unless a lot of keys are registered.
const byte SETUPCMD_ENABLE_INTERRUPTS=0xEA;

State static_state;
int static_runs = 0;

//Wait time between updates.
void set_debounce(State* state,unsigned long debounce) {
  state->debounce_micros = debounce;
  Serial.print("set debounce to ");
  Serial.print(((float)debounce)/1000.0);
  Serial.println("ms");
}

//Whether key state changes reset the debounce timer.
void set_await_smoothness(State* state,bool await) {
  state->await_smoothness=await;
  Serial.print("key state changes ");
  if (!await) {
    Serial.print("don't ");
  }
  Serial.println("reset the debounce timer");
}

//Whether interrupt handling is enabled.
bool set_enable_interrupts(State* state,bool enable) {
  state->enable_interrupts=enable;
  if (!enable) {
    Serial.print("not ");
  }
  Serial.println("listening to pin state change interrupts");
}

//Setup a key with the given pin.
void Key_setup(State* state,int pin) {
  //Add key to list
  if (state->key_count<MAX_KEYS) {
    //Set key properties
    Key* key = &state->keys[state->key_count];
    key->init(pin);
    //Log info
    Serial.print("added key ");
    Serial.print(state->key_count);
    Serial.print(" to pin ");
    Serial.println(pin);
    //Increase key count
    state->key_count++;
  }else{
    Serial.println("max key limit reached");
  }
}

//Check all keys for state changes
void Key_tick(State* state) {
  //Synchronize
  static volatile unsigned long min_tick_time = 0;
  noInterrupts();
  if (micros()<min_tick_time) {
    interrupts();
    return;
  }
  //Tick
  for(int i=0; i<state->key_count; i++) {
    Key* key = &state->keys[i];
    bool is_down = digitalRead(key->pin)==LOW;
    if (key->was_down!=is_down) {
      bool debounce = key->debounce_timer.check_now();
      //Notify host of the key state change only if debounce timer does not deny it
      if (!debounce) {
        //Send an event byte in the format `siii_iiii`, where `s` is the state bit and `i` is the index 7-bit unsigned key index
        byte event=(byte)( (i&0x7F) | (is_down? 0x80:0x00) );
        Serial.write(event);
        key->was_down = is_down;
      }
      //Update debounce timer if this message is in itself not debounced or 'await_smoothness' is enabled
      if (state->await_smoothness || !debounce) {
        key->debounce_timer.set( micros()+state->debounce_micros);
      }
    }
  }
  //Resume
  min_tick_time=micros()+50;
  interrupts();
}

void Key_handle_interrupt() {
  Key_tick(&static_state);
}

unsigned char next_serial_byte() {
  while(Serial.available()<1) {}
  return (unsigned char)(Serial.read());
}

void State_run(State* state) {
  state->init();

  //Read magic number from serial stream
  int magic_idx=0;
  int garbage=0;
  while(magic_idx<8) {
    if (next_serial_byte()!=MAGIC_NUMBER[magic_idx++]) {
      garbage+=magic_idx;
      magic_idx=0;
    }
  }
  
  //Send magic number
  Serial.write("SerKey01");
  Serial.print("read ");
  Serial.print(garbage);
  Serial.println(" bytes of garbage before magic number");
  Serial.print("run #");
  Serial.print(static_runs);
  Serial.println(" since last reset");
  Serial.println("handling setup commands");
  
  //Read commands
  bool process_commands = true;
  while (process_commands) {
    //Read command
    byte cmd=next_serial_byte();
    //Read payload length
    unsigned int payload_len=((unsigned int)next_serial_byte())<<8 | (unsigned int)next_serial_byte();
    //Check command
    switch(cmd) {
      //Stop handling commands
      case SETUPCMD_FINISH:
        Serial.println("finished handling setup commands");
        process_commands = false;
        break;
      //Setup a key
      case SETUPCMD_ADD_KEY:
        if (payload_len<1) {
          Serial.println("ADD_KEY command too short");
        }else{
          Key_setup(state,(int)next_serial_byte());
          payload_len-=1;
        }
        break;
      //Set debounce time
      case SETUPCMD_SET_DEBOUNCE:
        if (payload_len<4) {
          Serial.println("SET_DEBOUNCE command too short");
        }else{
          unsigned long debounce =
            ((unsigned long)next_serial_byte())<<24 |
            ((unsigned long)next_serial_byte())<<16 |
            ((unsigned long)next_serial_byte())<< 8 |
            ((unsigned long)next_serial_byte());
          set_debounce(state,debounce);
          payload_len-=4;
        }
        break;
      //Enable smoothness debounce
      case SETUPCMD_AWAIT_SMOOTHNESS:
        if (payload_len<1) {
          Serial.println("AWAIT_SMOOTHNESS command too short");
        }else{
          set_await_smoothness(state,(bool)next_serial_byte());
          payload_len-=1;
        }
        break;
      //Reset device, looping back to connection
      case CMD_RESET:
        //Currently unimplemented
        Serial.println("reset requested");
        goto quit;
        break;
      //Enable interrupt listening
      case SETUPCMD_ENABLE_INTERRUPTS:
        if (payload_len<1) {
          Serial.println("ENABLE_INTERRUPTS command too short");
        }else{
          set_enable_interrupts(state,(bool)next_serial_byte());
          payload_len-=1;
        }
        break;
      default:
        Serial.print("unknown command: ");
        Serial.println(cmd);
        break;
    }
    //Read excess bytes
    while(payload_len>0) {
      next_serial_byte();
      payload_len--;
    }
  }
  
  //Open pins
  for(int i=0; i<state->key_count; i++) {
    Key* key=&state->keys[i];
    //Check this pin hasn't already been setup
    bool already_setup=false;
    for(int j=0; j<i; j++) {
      if (state->keys[j].pin==key->pin) {
        already_setup=true;
        break;
      }
    }
    //Only setup if not already setup (duh)
    if (!already_setup) {
      Serial.print("opening pin ");
      Serial.println(key->pin);
      pinMode(key->pin,INPUT_PULLUP);
      if (state->enable_interrupts) {
        attachInterrupt(digitalPinToInterrupt(key->pin),&Key_handle_interrupt,CHANGE);
      }
    }
  }
  
  //Finish off log
  Serial.println("finished setting up, handling input");
  Serial.println();
  
  //Handle input in a loop
  while(true) {
    Key_tick(state);
    //Check if we should quit
    if (Serial.read() == CMD_RESET) {
      goto quit;
    }
  }
  quit:;
  
  //Cleanup
  for(int i=0; i<state->key_count; i++) {
    Key* key=&state->keys[i];
    pinMode(key->pin,INPUT);
    detachInterrupt(digitalPinToInterrupt(key->pin));
  }
}

void setup() {
  //Initialize serial communication
  Serial.begin(115200);
}

void loop() {
  //Run state
  //Every `loop` iteration is a single virtual reset
  static_runs++;
  State_run(&static_state);
}
