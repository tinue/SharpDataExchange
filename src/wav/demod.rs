//! Samples → timed signal edges, independent of the tape format.
//!
//! The decoders only ever look at *durations* between edges, so everything level- and
//! shape-related is handled here: DC / rumble removal, an envelope-relative hysteresis
//! comparator (works at any volume, ignores noise near zero), and sub-sample edge times
//! from linear interpolation of the zero crossing (needed at 5 kHz, where a 2500 Hz
//! half cycle is a single sample).

/// Alternating signal edges, in seconds from the start of the file.
pub struct Edges {
    pub times: Vec<f64>,
    /// Peak level of the (filtered) signal, dBFS (99th percentile, not a lone spike).
    pub level_dbfs: f64,
}

pub fn edges(samples: &[f32], rate: u32) -> Edges {
    let dt = 1.0 / rate as f64;
    // One-pole high-pass at 40 Hz: removes DC offset and motor rumble, keeps 1.2 kHz+.
    let rc = 1.0 / (2.0 * std::f64::consts::PI * 40.0);
    let a = rc / (rc + dt);
    let mut y = Vec::with_capacity(samples.len());
    let (mut yp, mut xp) = (0.0f64, samples.first().copied().unwrap_or(0.0) as f64);
    for &x in samples {
        let x = x as f64;
        yp = a * (yp + x - xp);
        xp = x;
        y.push(yp);
    }

    let reference = percentile_abs(&y, 0.99);
    let level_dbfs = if reference > 0.0 { 20.0 * reference.log10() } else { f64::NEG_INFINITY };
    // Below 2 % of the file's own level counts as silence: no edges from hiss.
    let gate = reference * 0.02;
    let decay = (-1.0 / (0.010 * rate as f64)).exp();

    let mut times = Vec::new();
    let mut env = 0.0f64;
    let mut state = 0i8; // +1 high, -1 low, 0 unknown
    let (mut last_up, mut last_down) = (None::<f64>, None::<f64>);
    for i in 0..y.len() {
        let v = y[i];
        env = (env * decay).max(v.abs());
        if i > 0 {
            let p = y[i - 1];
            if p <= 0.0 && v > 0.0 {
                last_up = Some(interp(i, p, v) * dt);
            } else if p >= 0.0 && v < 0.0 {
                last_down = Some(interp(i, p, v) * dt);
            }
        }
        let h = (env * 0.25).max(gate);
        let now = i as f64 * dt;
        let prev = times.last().copied().unwrap_or(f64::NEG_INFINITY);
        if state <= 0 && v > h {
            if state < 0 {
                times.push(last_up.filter(|&t| t > prev).unwrap_or(now));
            }
            state = 1;
        } else if state >= 0 && v < -h {
            if state > 0 {
                times.push(last_down.filter(|&t| t > prev).unwrap_or(now));
            }
            state = -1;
        }
    }
    Edges { times, level_dbfs }
}

/// Fractional sample index of the zero crossing between samples `i - 1` (value `p`)
/// and `i` (value `v`).
fn interp(i: usize, p: f64, v: f64) -> f64 {
    let d = v - p;
    let frac = if d != 0.0 { -p / d } else { 0.5 };
    (i - 1) as f64 + frac.clamp(0.0, 1.0)
}

fn percentile_abs(y: &[f64], q: f64) -> f64 {
    if y.is_empty() {
        return 0.0;
    }
    // A strided subsample is plenty for a level estimate and keeps this O(n).
    let step = (y.len() / 200_000).max(1);
    let mut v: Vec<f64> = y.iter().step_by(step).map(|s| s.abs()).collect();
    let k = ((v.len() - 1) as f64 * q) as usize;
    let (_, nth, _) = v.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
    *nth
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_wave_at_two_samples_per_cycle() {
        // 2500 Hz at 5 kHz: +A, -A, +A, ... — every sample is an edge.
        let s: Vec<f32> = (0..1000).map(|i| if i % 2 == 0 { 0.5 } else { -0.5 }).collect();
        let e = edges(&s, 5000);
        let d: Vec<f64> = e.times.windows(2).map(|w| w[1] - w[0]).collect();
        let mid = &d[100..900];
        assert!(mid.iter().all(|x| (x - 0.0002).abs() < 1e-6), "{:?}", &mid[..8]);
    }

    #[test]
    fn quiet_sine_gives_exact_half_periods() {
        let rate = 44100;
        let f = 1234.0;
        let s: Vec<f32> = (0..44100)
            .map(|i| (0.003 * (2.0 * std::f64::consts::PI * f * i as f64 / rate as f64).sin()) as f32)
            .collect();
        let e = edges(&s, rate);
        let d: Vec<f64> = e.times.windows(2).map(|w| w[1] - w[0]).collect();
        let half = 0.5 / f;
        assert!(d[200..2000].iter().all(|x| (x - half).abs() < half * 0.01));
        assert!(e.level_dbfs < -45.0);
    }
}
