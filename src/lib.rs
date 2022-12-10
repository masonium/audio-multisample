use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{
    BufferSize, BuildStreamError, Data, Device, InputCallbackInfo, PauseStreamError,
    PlayStreamError, SampleFormat, SampleRate, StreamConfig, StreamError,
};
use std::ops::DerefMut;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use serde::Deserialize;

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


#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NoteCaptureSettings {
    time_on: Duration,
    time_release: Duration,
    time_between: Duration,
    channels: u8,
    sample_rate: usize,
    midi_channel: u8,
    note_on_velocity: u8,
    note_off_velocity: u8,

    first_note: u8,
    last_note: u8,
    note_spacing: u8,
}

pub struct NoteCapturer<'d> {
    device: &'d Device,
    settings: NoteCaptureSettings,
}

impl Default for NoteCaptureSettings {
    fn default() -> Self {
	Self {
            time_on: Duration::from_secs_f32(0.02),
            time_release: Duration::from_secs_f32(0.02),
	    time_between: Duration::from_secs_f32(1.0),
            channels: 1,
            sample_rate: 44100,
            midi_channel: 1,
            note_on_velocity: 64,
            note_off_velocity: 64,

            first_note: 21,
            last_note: 108,
            note_spacing: 1,
	}
    }
}

impl NoteCaptureSettings {
    /// Return the buffer size to allocate for the number of samples
    /// needed to store each note.
    fn num_samples(&self) -> usize {
        let num_channels: u16 = self.channels as u16;
        let total_length_secs: f32 =
            self.time_on.as_secs_f32() + self.time_release.as_secs_f32() + 0.01;
        ((self.sample_rate * num_channels as usize) as f32 * total_length_secs) as usize
    }

    /// Return true iff all settings are valid.
    fn verify(&self) -> bool {
	if self.channels < 1 || self. channels > 2 {
	    return false;
	}

	true
    }
}
    

impl<'d> NoteCapturer<'d> {
    /// Return a new note capturer with standard settings.
    pub fn new(input_device: &Device) -> NoteCapturer {
        NoteCapturer {
            device: input_device,
	    settings: NoteCaptureSettings::default()
        }
    }

    pub fn apply_config(&mut self, config: &NoteCaptureSettings) {
	if config.verify() {
	    self.settings = config.clone();
	}
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
    pub fn capture_notes(
        &self,
        midi: &mut MidiOutputConnection,
    ) -> Result<Vec<NoteSample>, CaptureError> {
        let mut notes: Vec<u8> = (self.settings.first_note..=self.settings.last_note)
            .enumerate()
            .filter_map(|(i, n)| {
                if i as u8 % self.settings.note_spacing == 0 {
                    Some(n)
                } else {
                    None
                }
            })
            .collect();

	if notes.len() > 0 {
	    if notes[notes.len() - 1] != self.settings.last_note {
		notes.push(self.settings.last_note);
	    }
	}

        self.capture_note_list(midi, &notes)
    }

    /// Capture a list of notes in order.
    fn capture_note_list(
        &self,
        midi: &mut MidiOutputConnection,
        notes: &[u8],
    ) -> Result<Vec<NoteSample>, CaptureError> {
        let max_size = self.settings.num_samples();
        let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));

        let mut note_buffers = Vec::new();

        let in_config = StreamConfig {
            channels: self.settings.channels.into(),
            sample_rate: SampleRate(self.settings.sample_rate as u32),
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
                    self.settings.midi_channel,
                    *note,
                    self.settings.note_on_velocity,
                ))?;
                stream.play()?;
                std::thread::sleep(self.settings.time_on);
                midi.send(&Self::midi_note_off_message(
                    self.settings.midi_channel,
                    *note,
                    self.settings.note_off_velocity,
                ))?;
                std::thread::sleep(self.settings.time_release);
                stream.pause()?;
            }
            {
                if error.lock().unwrap().is_some() {
                    let lock = Arc::try_unwrap(error).expect("should be no lock");
                    let stream_error = lock.into_inner().expect("should not be poisoned").unwrap();
                    return Err(CaptureError::Stream(stream_error));
                }
            }

            let mut ret_buf: Vec<f32> = Vec::new();
            let mut raw_buf = buffer.lock().unwrap();
            std::mem::swap(raw_buf.deref_mut(), &mut ret_buf);

            note_buffers.push(ret_buf);
	    std::thread::sleep(self.settings.time_between);
        }

        Ok(note_buffers)
    }
}
