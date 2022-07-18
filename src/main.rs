use anyhow::Result;
use audio_multisample::NoteCapturer;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Data, Host, OutputCallbackInfo, SampleFormat, SampleRate, StreamConfig, StreamError,
};
use midir::{MidiIO, MidiOutput, MidiOutputPort};
use std::sync::{Arc, Mutex};

fn error_callback(e: StreamError) {
    println!("{:?}", e);
}

fn play_sine(host: &Host) -> Result<()> {
    let output = host.default_output_device().unwrap();
    let audio_buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));

    let config = StreamConfig {
        channels: 2,
        sample_rate: SampleRate(44100),
        buffer_size: BufferSize::Default,
    };

    let output_buffer: Arc<Vec<_>> = Arc::new(audio_buffer.lock().unwrap().clone());

    let mut ci: usize = 0;
    let freq = 440;
    let sample_rate = 44100;
    let mult1: f32 = freq as f32 / sample_rate as f32 * 2.0 * std::f32::consts::PI;
    let mult2: f32 = mult1 * 2.0_f32.powf(7.0 / 12.0);
    let callback = move |d: &mut Data, _oci: &OutputCallbackInfo| {
        if let Some(buf) = d.as_slice_mut() {
            println!("{}", buf.len());
            //let (b1, b2) = buf.split_at_mut(buf.len() / 2);
            for x in buf.chunks_mut(2) {
                x[0] = (mult1 * (ci as f32)).sin();
                x[1] = (mult2 * (ci as f32)).sin();
                ci += 1;
            }
        }
    };
    let stream =
        output.build_output_stream_raw(&config, SampleFormat::F32, callback, error_callback)?;
    stream.play().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1000));
    stream.pause().unwrap();

    Ok(())
}

fn main() -> Result<()> {
    let host = cpal::default_host();
    let device = host.default_input_device().unwrap();

    let capturer = NoteCapturer::new(&device);
    let midi_out = MidiOutput::new("audio_multisample")?;

    let mut port = None;
    for out_port in midi_out.ports() {
        println!("{}", midi_out.port_name(&out_port)?);
	port = Some(out_port);
    }

    let port = port.unwrap();
    let mut conn = midi_out.connect(&port, "sampler").unwrap();

    let res = capturer.capture_note(&mut conn, 60)?;

    Ok(())
}
