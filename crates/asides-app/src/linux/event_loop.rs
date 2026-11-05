//! The event loop on Linux: the little of `tao` the app uses, on a channel. tao would bring GTK, for
//! windows the app doesn't have: its windows are the overlay's process, and all it waits on here is
//! what its own threads send, and the next deadline.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Instant;

pub mod event {
    #![allow(clippy::enum_variant_names, reason = "tao's names")]
    use std::marker::PhantomData;

    /// Why the loop woke.
    pub enum StartCause {
        Init,
        ResumeTimeReached {},
    }

    pub enum Event<'a, T> {
        NewEvents(StartCause),
        /// macOS's: links opened through Launch Services.
        #[allow(dead_code)]
        Opened {
            urls: Vec<url::Url>,
        },
        UserEvent(T),
        LoopDestroyed,
        /// tao's events borrow; none here does.
        #[allow(dead_code)]
        Borrowing(PhantomData<&'a ()>),
    }
}

pub mod event_loop {
    use super::*;

    pub enum ControlFlow {
        Wait,
        WaitUntil(Instant),
        Exit,
    }

    /// The other end of the loop's channel.
    pub struct EventLoopProxy<T>(Sender<T>);

    impl<T> Clone for EventLoopProxy<T> {
        fn clone(&self) -> Self {
            EventLoopProxy(self.0.clone())
        }
    }

    /// The event loop has ended.
    #[derive(Debug)]
    pub struct EventLoopClosed;

    impl<T> EventLoopProxy<T> {
        pub fn send_event(&self, event: T) -> Result<(), EventLoopClosed> {
            self.0.send(event).map_err(|_| EventLoopClosed)
        }
    }

    pub struct EventLoopBuilder<T>(std::marker::PhantomData<T>);

    impl<T> EventLoopBuilder<T> {
        pub fn with_user_event() -> Self {
            EventLoopBuilder(std::marker::PhantomData)
        }

        pub fn build(self) -> EventLoop<T> {
            let (events, received) = channel();
            EventLoop { events, received }
        }
    }

    pub struct EventLoop<T> {
        events: Sender<T>,
        received: Receiver<T>,
    }

    impl<T> EventLoop<T> {
        pub fn create_proxy(&self) -> EventLoopProxy<T> {
            EventLoopProxy(self.events.clone())
        }

        /// Runs `handler` on each event until it says to stop; the process ends then.
        pub fn run(self, mut handler: impl FnMut(event::Event<'_, T>, &(), &mut ControlFlow)) -> ! {
            use event::{Event, StartCause};
            let mut flow = ControlFlow::Wait;
            handler(Event::NewEvents(StartCause::Init), &(), &mut flow);
            loop {
                let event = match flow {
                    ControlFlow::Exit => {
                        handler(Event::LoopDestroyed, &(), &mut flow);
                        std::process::exit(0);
                    }
                    ControlFlow::Wait => self.received.recv().ok().map(Event::UserEvent),
                    ControlFlow::WaitUntil(at) => {
                        match self.received.recv_timeout(at.saturating_duration_since(Instant::now())) {
                            Ok(event) => Some(Event::UserEvent(event)),
                            Err(RecvTimeoutError::Timeout) => Some(Event::NewEvents(StartCause::ResumeTimeReached {})),
                            Err(RecvTimeoutError::Disconnected) => None,
                        }
                    }
                };
                // The loop keeps a sender itself: its channel never closes.
                let Some(event) = event else { continue };
                handler(event, &(), &mut flow);
            }
        }
    }
}
