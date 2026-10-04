//! Tone synthesis for the encoders: whole cycles of a frequency, rendered at any sample
//! rate from an exact time line (no drift, no rounding of bit lengths), phase-continuous
//! because every tone is a whole number of cycles.

/// Peak amplitude: −3 dBFS.
const AMP: f64 = 0.707;
/// Edge sharpness of the soft-square wave shape (0 would be a sine). Cassette inputs
/// trigger on zero crossings; steeper crossings survive a PC's output stage better,
/// while the rounding keeps the spectrum clean.
const SHAPE: f64 = 2.5;
/// Fade in / out at the edges of every tone burst, seconds.
const FADE: f64 = 0.002;

pub(super) struct Synth {
    rate: f64,
    out: Vec<f32>,
    /// Exact time the next tone starts, seconds.
    t: f64,
    /// Sample index where the current tone burst began (for the fade-in).
    burst_start: Option<usize>,
}

impl Synth {
    pub fn new(rate: u32) -> Self {
        Synth { rate: rate as f64, out: Vec::new(), t: 0.0, burst_start: None }
    }

    /// `cycles` whole cycles of `freq` Hz.
    pub fn tone(&mut self, freq: f64, cycles: f64) {
        let t0 = self.t;
        let t1 = t0 + cycles / freq;
        if self.burst_start.is_none() {
            self.burst_start = Some(self.out.len());
        }
        let norm = SHAPE.tanh();
        let mut n = self.out.len();
        while (n as f64) / self.rate < t1 {
            // A quarter-cycle offset puts samples on the peaks, not the zero
            // crossings, when the rate is only twice the tone (2500 Hz at 5 kHz).
            let phase = freq * ((n as f64) / self.rate - t0) + 0.25;
            let s = (SHAPE * (2.0 * std::f64::consts::PI * phase).sin()).tanh() / norm;
            self.out.push((AMP * s) as f32);
            n += 1;
        }
        self.t = t1;
    }

    /// Silence; fades the preceding tone burst in and out.
    pub fn silence(&mut self, secs: f64) {
        self.close_burst();
        self.t += secs;
        let end = (self.t * self.rate).ceil() as usize;
        if end > self.out.len() {
            self.out.resize(end, 0.0);
        }
    }

    fn close_burst(&mut self) {
        let Some(start) = self.burst_start.take() else {
            return;
        };
        let fade = ((FADE * self.rate) as usize).max(1);
        let len = self.out.len() - start;
        let fade = fade.min(len / 2);
        for k in 0..fade {
            let g = k as f32 / fade as f32;
            self.out[start + k] *= g;
            let e = self.out.len() - 1 - k;
            self.out[e] *= g;
        }
    }

    pub fn finish(mut self) -> Vec<f32> {
        self.close_burst();
        self.out
    }
}
