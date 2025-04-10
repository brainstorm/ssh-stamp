// Inspired by https://play.rust-lang.org/?version=stable&mode=debug&edition=2015&gist=ee3e4df093c136ced7b394dc7ffb78e1
// Originally described in https://hoverbear.org/blog/rust-state-machine-pattern/

// Tenets:
//  1. Lightweight and easy to understand/change.
//  2. Should not interfere in performance, only "big" state transitions should be tracked (not micromanage on bytes sent, etc...).
//  3. Non intrusive in application code.

pub enum State {
    PowerOn,
    Reset,
    Idle,
    Timeout,
    BridgeUp,
    InitPeripherals,
    TcpStackUp,
    TaskSpawning { name: &str },
    TaskFailed { name: &str },
    TaskRunning { name: &str },
    AllTasksOk,
    ClientConnecting,
    ClientConnected,
    AuthzChecks,
    SshConnEstablished,
    ReadEnvVars,
    UartReconf,
    SshUartBridgeEstablished,
}

enum Event {
    Ok,
    Fail,
    UartReconf,
    ClientConnect,
    SshDisconnect,
}

impl State {
    fn next(self, event: Event) -> State {
        match (self, event) {
            (State::PowerOn, Event::Ok) => State::TaskSpawning,
            (State::TaskSpawning { .. }, Event::Ok) => State::TaskRunning { .. },
            (State::AllTasksOk, Event::Ok) => State::BridgeUp,
            (State::BridgeUp, Event::Ok) => State::Idle,
            (State::Idle, Event::Ok) => State::Idle,
            (State::Idle, Event::ClientConnect) => State::ClientConnecting,
            (State::ClientConnecting, Event::Ok) => State::ReadEnvVars,
            (State::ReadEnvVars, Event::Ok) => State::ClientConnected,
            (State::ClientConnected, Event::Ok) => State::SshUartBridgeEstablished,
            (s, e) => {
                State::Fail(println!("Wrong state, event combination: {:#?} {:#?}", s, e))
            }
        }
    }
    fn run(&self) {
        match *self {
            State::Idle |
            State::Fail(_) => {}
        }
    }
}

// fn main() {
//     let mut state = State::Idle;
//
//     // Sequence of events (might be dynamic based on what State::run did)
//     // TODO: Declare this array automatically from the enum definition above.
//     let events = [Event::Ok,
//                   Event::Fail];
//
//     let mut iter = events.iter();
//
//     loop {
//         // just a hack to get owned values, because I used an iterator
//         let event = iter.next().unwrap().clone();
//         print!("__ Transition from {:?}", state);
//         state = state.next(event);
//         println!(" to {:?}", state);

//         if let State::Fail(string) = state {
//             println!("{}", string);
//             break;
//         } else {
//             // You might want to do somethin while in a state
//             // You could also add State::enter() and State::exit()
//             state.run();
//         }
//     }

// }