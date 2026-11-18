//! One sound file played on its own: the library page's ▶, as LibraryPage.svelte's `<audio>`.
//! The same decoder and sound card as the preview's ([`super::audio`]); dropping it stops it.

use super::audio::{Output, Source, Span};
use crate::state::ServerAddress;

/// Longer than any sound file in a library; ffmpeg stops at the file's end.
const LONGEST: f64 = 24. * 60. * 60.;

pub struct Sound {
    output: Output,
}

impl Sound {
    /// Plays `url` (on `server`) from the start.
    pub fn play(server: &ServerAddress, url: &str) -> anyhow::Result<Sound> {
        let output = Output::open()?;
        let mixer = &output.mixer;
        let span = Span {
            seek: 0.,
            length: LONGEST,
            speed: 1.,
            first: mixer.sample(0.),
        };
        mixer.seek(0.);
        mixer.add(Source::decode(
            server,
            "sound".into(),
            url,
            span,
            mixer.rate,
        )?);
        mixer.set_playing(true);
        Ok(Sound { output })
    }

    /// It played to its end (or couldn't be read).
    pub fn finished(&self) -> bool {
        self.output.mixer.finished()
    }
}
