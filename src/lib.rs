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

    #[error("error during stream capture")]
    Stream(#[from] StreamError),

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

    first_note: u8,
    last_note: u8,
    note_spacing: u8
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
	    note_off_velocity: 64,

	    first_note: 21,
	    last_note: 108,
	    note_spacing: 1
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

    /// Capture the collection of notes in the range from first_note to last_note.
    pub fn capture_notes(&self,
			 midi: &mut MidiOutputConnection
    ) -> Result<Vec<NoteSample>, CaptureError> {
	let notes: Vec<u8> = (self.first_note..=self.last_note)
	    .enumerate()
	    .filter_map(|(i, n)| {
		if i as u8 % self.note_spacing == 0 {
		    Some(n)
		} else {
		    None
		}})
	    .collect();
	
	self.capture_note_list(midi, &notes)
    }

    /// Capture a list of notes in order.
    fn capture_note_list(
        &self,
        midi: &mut MidiOutputConnection,
        notes: &[u8],
    ) -> Result<Vec<NoteSample>, CaptureError> {
        let max_size = self.num_samples();
        let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));

	let mut note_buffers = Vec::new();

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

	let stream = self.device.build_input_stream_raw(
	    &in_config,
	    SampleFormat::F32,
	    data_callback,
	    error_callback,
	)?;

	for note in notes {
	    {
		let mut b = buffer.lock().unwrap();
		b.clear();
		b.reserve(max_size);
	    }

	    {
		midi.send(&Self::midi_note_on_message(
		    self.midi_channel,
		    *note,
		    self.note_on_velocity,
		))?;
		stream.play()?;
		std::thread::sleep(self.length_on);
		midi.send(&Self::midi_note_off_message(
		    self.midi_channel,
		    *note,
		    self.note_off_velocity,
		))?;
		std::thread::sleep(self.length_release);
		stream.pause()?;
	    }
	    {
		if error.lock().unwrap().is_some() {
		    let lock = Arc::try_unwrap(error).expect("should be no lock");
		    let stream_error = lock.into_inner()
			.expect("should not be poisoned")
			.unwrap();
		    return Err(CaptureError::Stream(stream_error));
		}
	    }

	    let mut ret_buf: Vec<f32> = Vec::new();
	    let mut raw_buf = buffer.lock().unwrap();
	    std::mem::swap(raw_buf.deref_mut(), &mut ret_buf);

	    note_buffers.push(ret_buf);
	}

	Ok(note_buffers)
    }
}
