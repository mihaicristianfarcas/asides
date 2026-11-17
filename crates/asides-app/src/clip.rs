//! A clip being recorded (`asides_core::motion`): what the app in front shows over the live screen,
//! from the pill's record button until it's pressed again, the screen freezes, or the aside is sent
//! or closed. The frames come from `capture::watch`, on a thread of its own, straight into the
//! recorder (which keeps few); the main thread only starts and stops it. Stopped, it's put together
//! off the main thread (`Recording::finish`): its pictures' PNGs take a moment.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use asides_core::measure::{Frame, Rect};
use asides_core::{motion, picture};

use crate::capture;

/// What a watch tells a clip.
pub enum Shown {
    Frame(Frame),
    /// Another app's window is in front now (the app's name), or the window is another size: the
    /// clip starts over, on that one, leaving these parts of its frames as they first are (asides'
    /// own pill over it, where a frame has it). Where the watch itself follows the window in front
    /// (Windows, Linux); on a Mac, the app hears of the app in front and starts another clip.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Another(Option<String>, Vec<Rect>),
}

/// What takes a watch's frames.
pub type Take = Box<dyn Fn(Shown) + Send + Sync>;

/// What's recorded so far: of which app, since when, and from its first frame on (its times count
/// from when that came), what its recorder keeps.
struct Tape {
    app: Option<String>,
    /// Parts of its frames that aren't the user's, in a frame's pixels as the watch gives it.
    ignored: Vec<Rect>,
    started: Instant,
    recorder: Option<(Instant, motion::Recorder)>,
}

impl Tape {
    fn new(app: Option<String>, ignored: Vec<Rect>) -> Tape {
        Tape { app, ignored, started: Instant::now(), recorder: None }
    }

    /// The next frame: recorded no larger than `motion::SIDE` a side.
    fn push(&mut self, frame: Frame) {
        let longer = frame.width.max(frame.height);
        let (frame, scale) = match longer > motion::SIDE {
            true => (picture::within(&frame, motion::SIDE), f64::from(motion::SIDE) / f64::from(longer)),
            false => (frame, 1.0),
        };
        let (first, recorder) = self.recorder.get_or_insert_with(|| {
            let smaller = |r: &Rect| {
                let at = |v: u32| (f64::from(v) * scale).floor() as u32;
                let long = |v: u32| (f64::from(v) * scale).ceil() as u32 + 1;
                Rect::new(at(r.x), at(r.y), long(r.width), long(r.height))
            };
            (Instant::now(), motion::Recorder::ignoring(self.ignored.iter().map(smaller).collect()))
        });
        recorder.push(first.elapsed().as_secs_f32(), frame);
    }
}

/// A clip being recorded. Dropped, it's thrown away.
pub struct Recording {
    /// Taken when it stops: frames still on their way find none.
    tape: Arc<Mutex<Option<Tape>>>,
    watch: capture::Watch,
    /// The app it's of (0 where the watch follows the window in front).
    pub pid: i32,
    pub asked: Instant,
}

/// A clip, put together: when it started, its pictures (PNGs), what's said after them, and the app
/// it's of (said in that).
pub struct Made {
    pub started: Instant,
    pub pngs: Vec<Vec<u8>>,
    pub summary: motion::Summary,
    pub app: Option<String>,
}

impl Recording {
    /// Starts one of app `pid` (called `app`): `watch` starts the frames coming, to what it's given.
    pub fn start(pid: i32, app: Option<String>, watch: impl FnOnce(Take) -> capture::Watch) -> Recording {
        let tape = Arc::new(Mutex::new(Some(Tape::new(app, Vec::new()))));
        let taking = tape.clone();
        let watch = watch(Box::new(move |shown| {
            let mut tape = taking.lock().unwrap();
            let Some(tape) = tape.as_mut() else { return };
            match shown {
                Shown::Frame(frame) => tape.push(frame),
                Shown::Another(app, ignored) => *tape = Tape::new(app, ignored),
            }
        }));
        Recording { tape, watch, pid, asked: Instant::now() }
    }

    /// Stops it and puts it together. `None` when no frame came. Not for the main thread.
    pub fn finish(self) -> Option<Made> {
        let Recording { tape, watch, .. } = self;
        drop(watch);
        let tape = tape.lock().unwrap().take()?;
        let (first, recorder) = tape.recorder?;
        let clip = recorder.finish(first.elapsed().as_secs_f32())?;
        let pngs: Vec<Vec<u8>> = motion::pictures(&clip).iter().map(picture::png).collect();
        (!pngs.is_empty()).then(|| Made { started: tape.started, pngs, summary: clip.summary(), app: tape.app })
    }
}
