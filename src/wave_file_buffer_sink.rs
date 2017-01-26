#![allow(dead_code)]

use audio_buffer_sink::*;

use std::io::{self, Write, Seek, SeekFrom, BufWriter};
use std::fs::File;
use std::path::Path;

const NUM_CHANNELS: usize = 2;
const BITS_PER_SAMPLE: usize = 16;

pub struct WaveFileBufferSink {
    input_sample_rate: usize,
    output_sample_rate: usize,

    writer: BufWriter<File>,
    num_frames: usize,

    inner: Vec<i16>,
}

impl WaveFileBufferSink {
    pub fn new<P: AsRef<Path>>(file_name: P, input_sample_rate: usize) -> io::Result<WaveFileBufferSink> {
        let output_sample_rate = 48000;

        let file = File::create(file_name)?;
        let writer = BufWriter::new(file);

        let mut ret = WaveFileBufferSink {
            input_sample_rate: input_sample_rate,
            output_sample_rate: output_sample_rate,

            writer: writer,
            num_frames: 0,

            inner: Vec::new(),
        };

        // RIFF header
        ret.write_str("RIFF")?;
        ret.write_u32(0)?; // Data sub-chunk size; will be written properly later
        ret.write_str("WAVE")?;

        // Format sub-chunk
        ret.write_str("fmt ")?;
        ret.write_u32(16)?;
        ret.write_u16(1)?; // WAVE_FORMAT_PCM
        ret.write_u16(NUM_CHANNELS as _)?;
        ret.write_u32(output_sample_rate as _)?;
        ret.write_u32((output_sample_rate * NUM_CHANNELS * BITS_PER_SAMPLE / 8) as _)?;
        ret.write_u16((NUM_CHANNELS * BITS_PER_SAMPLE / 8) as _)?;
        ret.write_u16(BITS_PER_SAMPLE as _)?;

        // Data sub-chunk
        ret.write_str("data")?;
        ret.write_u32(0)?; // Data size; will be written properly later

        Ok(ret)
    }

    fn write_str(&mut self, value: &str) -> io::Result<()> {
        self.writer.write_all(value.as_bytes())?;

        Ok(())
    }

    fn write_u16(&mut self, value: u16) -> io::Result<()> {
        let buf = [value as u8, (value >> 8) as u8];

        self.writer.write_all(&buf)?;

        Ok(())
    }

    fn write_u32(&mut self, value: u32) -> io::Result<()> {
        let buf = [value as u8, (value >> 8) as u8, (value >> 16) as u8, (value >> 24) as u8];

        self.writer.write_all(&buf)?;

        Ok(())
    }
}

impl Drop for WaveFileBufferSink {
    fn drop(&mut self) {
        let resampler = LinearResampler::new(self.inner.clone().into_iter(), 4166666666, 4800000000);//self.input_sample_rate, self.output_sample_rate);

        for sample in resampler {
            let _ = self.write_u16(sample as _);
            self.num_frames += 1;
        }
        self.num_frames /= 2;

        let data_chunk_size = self.num_frames * NUM_CHANNELS * BITS_PER_SAMPLE / 8;

        let _ = self.writer.seek(SeekFrom::Start(4));
        let _ = self.write_u32((data_chunk_size + 36) as _); // Data sub-chunk size
        let _ = self.writer.seek(SeekFrom::Start(40));
        let _ = self.write_u32(data_chunk_size as _); // Data size
    }
}

impl AudioBufferSink for WaveFileBufferSink {
    fn append(&mut self, buffer: &[(i16, i16)]) {
        for &(left, right) in buffer {
            self.inner.push(left);
            self.inner.push(right);
        }
    }
}

struct LinearResampler<I> {
    inner: I,

    from_sample_rate: usize,
    to_sample_rate: usize,

    current_from_frame: (i16, i16),
    next_from_frame: (i16, i16),
    from_fract_pos: usize,

    current_frame_channel_offset: usize,
}

impl<I: Iterator<Item = i16>> LinearResampler<I> {
    fn new(inner: I, from_sample_rate: usize, to_sample_rate: usize) -> LinearResampler<I> {
        let sample_rate_gcd = {
            fn gcd(a: usize, b: usize) -> usize {
                if b == 0 {
                    a
                } else {
                    gcd(b, a % b)
                }
            }

            gcd(from_sample_rate, to_sample_rate)
        };

        LinearResampler {
            inner: inner,

            from_sample_rate: from_sample_rate / sample_rate_gcd,
            to_sample_rate: to_sample_rate / sample_rate_gcd,

            current_from_frame: (0, 0),
            next_from_frame: (0, 0),
            from_fract_pos: 0,

            current_frame_channel_offset: 0,
        }
    }
}

impl<I: Iterator<Item = i16>> Iterator for LinearResampler<I> {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        fn interpolate(a: i16, b: i16, num: usize, denom: usize) -> i16 {
            (((a as isize) * ((denom - num) as isize) + (b as isize) * (num as isize)) / (denom as isize)) as _
        }

        let ret = match self.current_frame_channel_offset {
            0 => interpolate(self.current_from_frame.0, self.next_from_frame.0, self.from_fract_pos, self.to_sample_rate),
            _ => interpolate(self.current_from_frame.1, self.next_from_frame.1, self.from_fract_pos, self.to_sample_rate)
        };

        self.current_frame_channel_offset += 1;
        if self.current_frame_channel_offset >= 2 {
            self.current_frame_channel_offset = 0;

            self.from_fract_pos += self.from_sample_rate;
            while self.from_fract_pos > self.to_sample_rate {
                self.from_fract_pos -= self.to_sample_rate;

                self.current_from_frame = self.next_from_frame;

                let left = self.inner.next().unwrap_or(0);
                let right = match self.inner.next() {
                    Some(x) => x,
                    _ => {
                        return None;
                    }
                };
                self.next_from_frame = (left, right);
            }
        }

        Some(ret)
    }
}