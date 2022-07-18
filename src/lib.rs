use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{
    BufferSize, BuildStreamError, Data, Device, InputCallbackInfo,
    SampleFormat, SampleRate, StreamConfig, StreamError, PauseStreamError, PlayStreamError,
};
use std::ops::DerefMut;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;

use midir::MidiOutputConnection;

pub type NoteSample = Vec<f32>;

#[derive(Error, Debug)]
pub enum CaptureError {
    #[error("could not build input stream")]
    BuildStream(#[from] BuildStreamError),

    #[error("could not pause input stream")]
    PauseStream(#[from] PauseStreamError),

    #[error("could not play input stream")]
    PlayStream(#[from] PlayStreamError),

    #[error("could not send midi message")]
    MidiSend(#[from] midir::SendError),
}

#[derive(Clone, Copy, Debug)]
pub enum CaptureChannels {
    Mono,
    Stereo,
}

impl From<CaptureChannels> for u16 {
    fn from(cc: CaptureChannels) -> Self {
        match cc {
            CaptureChannels::Mono => 1,
            CaptureChannels::Stereo => 2,
        }
    }
}

pub struct NoteCapturer<'d> {
    device: &'d Device,
    length_on: Duration,
    length_release: Duration,
    channels: CaptureChannels,
    sample_rate: usize,
    midi_channel: u8,
    note_on_velocity: u8,
    note_off_velocity: u8,
}

impl<'d> NoteCapturer<'d> {

    /// Return a new note capturer with standard settings.
    pub fn new(input_device: &Device) -> NoteCapturer {
	NoteCapturer {
	    device: input_device,
	    length_on: Duration::from_secs_f32(0.02),
	    length_release: Duration::from_secs_f32(0.02),
	    channels: CaptureChannels::Mono,
	    sample_rate: 44100,
	    midi_channel: 1,
	    note_on_velocity: 64,
	    note_off_velocity: 64
	}
    }

    fn num_samples(&self) -> usize {
        let num_channels: u16 = self.channels.into();
        let total_length_secs: f32 =
            self.length_on.as_secs_f32() + self.length_release.as_secs_f32() + 0.01;
        ((self.sample_rate * num_channels as usize) as f32 * total_length_secs) as usize
    }

    /// Return a raw byte array represnting a MIDI Note Off message.
    fn midi_note_off_message(channel: u8, note: u8, velocity: u8) -> [u8; 3] {
        [0x80 | (channel & 0xF), note, velocity]
    }

    /// Return a raw byte array represnting a MIDI Note On message.
    fn midi_note_on_message(channel: u8, note: u8, velocity: u8) -> [u8; 3] {
        [0x90 | (channel & 0xF), note, velocity]
    }

    pub fn capture_note(
        &self,
        midi: &mut MidiOutputConnection,
        note: u8,
    ) -> Result<NoteSample, CaptureError> {
        let max_size = self.num_samples();
        let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::with_capacity(max_size)));

        let in_config = StreamConfig {
            channels: self.channels.into(),
            sample_rate: SampleRate(self.sample_rate as u32),
            buffer_size: BufferSize::Default,
        };

        let error = Arc::new(Mutex::new(None));
        let e2 = error.clone();
        let error_callback = move |e: StreamError| {
            *e2.lock().unwrap() = Some(e);
        };

        let b2 = buffer.clone();
        let mut total_num_samples = 0;
        let data_callback = move |d: &Data, _ici: &InputCallbackInfo| {
            let mut buf = b2.lock().unwrap();
            for s in d.as_slice().unwrap() {
                if total_num_samples >= max_size {
                    break;
                }
		buf.push(*s);
                total_num_samples += 1;
            }
        };

	{
            let stream = self.device.build_input_stream_raw(
		&in_config,
		SampleFormat::F32,
		data_callback,
		error_callback,
            )?;
            midi.send(&Self::midi_note_on_message(
		self.midi_channel,
		note,
		self.note_on_velocity,
            ))?;
            stream.play()?;
	    std::thread::sleep(self.length_on);
	    midi.send(&Self::midi_note_on_message(
		self.midi_channel,
		note,
		self.note_off_velocity,
            ))?;
	    std::thread::sleep(self.length_release);
	    stream.pause()?;
	}
	
	let mutex_buf = Arc::try_unwrap(buffer).unwrap();
	let mut ret_buf: Vec<f32> = Vec::new();
	let mut raw_buf = mutex_buf.lock().unwrap();

	std::mem::swap(raw_buf.deref_mut(), &mut ret_buf);
	Ok(ret_buf)
    }
}
