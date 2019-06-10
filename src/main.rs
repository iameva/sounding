extern crate gl;
extern crate glutin;
extern crate chrono;
extern crate byteorder;

mod support;

use cpal;
use glutin::Context;
use gl::types::*;
use std::mem;
use std::ptr;
use std::str;
use std::os::raw::c_void;
use std::ffi::CString;
use std::ffi::CStr;
use std::sync::atomic::AtomicBool;
use std::path::{Path, PathBuf};
use std::fs::File;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::env;
use std::io::prelude::*;
use byteorder::{BigEndian, WriteBytesExt, ReadBytesExt};
use winit::{ElementState, VirtualKeyCode};


use chrono::prelude::*;
use std::env::args;
use std::process::exit;


const SCREEN_WIDTH: f64 = 800.0;
const SCREEN_HEIGHT: f64 = 600.0;

enum RecordingEvent {
    Start(PathBuf),
    Stop(),
    Data(Vec<f32>),
}

fn main() {
    match args().skip(1).next() {
        Some(path) => play(path),
        _ => record()
    }
}

fn record() {
    let base_path = {
        let mut base = env::current_dir().unwrap();
        base.push("recordings");
        base.push(Local::now().format("%Y-%m-%d-%H:%M:%S").to_string());
        base
    };
    std::fs::create_dir_all(base_path.clone());

    let local = Local::now();

    // initialize the audio stuffs
    let event_loop = cpal::EventLoop::new();

    // Default devices.
    let input_device = cpal::default_input_device().expect("Failed to get default input device");
    let output_device = cpal::default_output_device().expect("Failed to get default output device");
    println!("Using default input device: \"{}\"", input_device.name());
    println!("Using default output device: \"{}\"", output_device.name());

    // We'll try and use the same format between streams to keep it simple
    let mut format = input_device.default_output_format().expect("Failed to get default format");
    format.data_type = cpal::SampleFormat::F32;

    // Build streams.
    println!("Attempting to build both streams with `{:?}`.", format);
    let input_stream_id = event_loop.build_input_stream(&input_device, &format).unwrap();
    let output_stream_id = event_loop.build_output_stream(&output_device, &format).unwrap();
    println!("Successfully built streams.");

    let recording = Arc::new(AtomicBool::new(false));

    // The channel to share samples.
    let (writerTx, writerRx) = std::sync::mpsc::sync_channel(1024);

    std::thread::spawn(move || {
        let mut file = None;
        loop {
            match writerRx.recv().unwrap() {
                RecordingEvent::Start(path) => {
                    println!("Starting recording on path {:?}", path);
                    file = Some(File::create(path).unwrap());
                }
                RecordingEvent::Stop() => {
                    file = None
                }
                RecordingEvent::Data(data) => {
                    match file {
                        Some(ref mut f) => {
                            data.into_iter().for_each(|d| {
                                f.write_f32::<BigEndian>(d);
                            });
                        }
                        None => {
                            println!("Received data when not recording");
                        }
                    }
                }
            }
        }
    });

    event_loop.play_stream(input_stream_id.clone());
//    event_loop.play_stream(output_stream_id.clone());

    // Run the event loop on a separate thread.
    {
        let recording = recording.clone();
        std::thread::spawn(move || {
            let mut was_recording = false;
            let mut count = 0;
            event_loop.run(move |id, data| {
                match data {
                    cpal::StreamData::Input { buffer: cpal::UnknownTypeInputBuffer::F32(buffer) } => {
                        assert_eq!(id, input_stream_id);
                        if recording.load(Relaxed) {
                            if !was_recording {
                                let mut path = base_path.to_path_buf();
                                path.push(format!("{:06}", count));
                                writerTx.send(RecordingEvent::Start(path));
                                was_recording = true;
                                count += 1;
                            }
                            writerTx.send(RecordingEvent::Data(buffer.to_vec()));
                            // send data
                        } else {
                            if was_recording {
                                was_recording = false;
                                writerTx.send(RecordingEvent::Stop());
                            }
                        };
                    }
                    cpal::StreamData::Output { buffer: cpal::UnknownTypeOutputBuffer::F32(mut buffer) } => {

                    }
                    _ => panic!("we're expecting f32 data"),
                }
            });
        });
    }


    // initialize the gl window and event loop
    let mut el = glutin::EventsLoop::new();
    let wb = glutin::WindowBuilder::new().with_title("A fantastic window!");

    let windowed_context = glutin::ContextBuilder::new()
        .build_windowed(wb, &el)
        .unwrap();

    let windowed_context = unsafe { windowed_context.make_current().unwrap() };

    println!(
        "Pixel format of the window's GL context: {:?}",
        windowed_context.get_pixel_format()
    );

    let gl = support::load(&windowed_context.context());

    let mut running = true;
    while running {
        el.poll_events(|event| {
            match event {
                glutin::Event::DeviceEvent { event, .. } => match event {
                    glutin::DeviceEvent::Key(input) => {
                        match input.virtual_keycode {
                            Some(VirtualKeyCode::R) => {
                                if input.state == ElementState::Pressed {
                                    recording.store(true, Relaxed);
                                } else {
                                    recording.store(false, Relaxed);
                                }
                            }
                            Some(VirtualKeyCode::Escape) => {
                                if input.state == ElementState::Released {
                                    running = false;
                                }
                            }
                            _ => ()
                        }
                    }
                    _ => ()
                },
                glutin::Event::WindowEvent { event, .. } => match event {
                    glutin::WindowEvent::CloseRequested => running = false,
                    glutin::WindowEvent::Resized(logical_size) => {
                        let dpi_factor =
                            windowed_context.window().get_hidpi_factor();
                        windowed_context
                            .resize(logical_size.to_physical(dpi_factor));
                    }
                    _ => (),
                },
                _ => (),
            }
        });

        let color = if recording.load(Relaxed) {
            [1.0, 0.0, 0.0, 1.0]
        } else {
            [0.2, 0.2, 0.8, 1.0]
        };
        gl.draw_frame(color);
        windowed_context.swap_buffers().unwrap();
    }
}

fn play(path: String) {
    println!("playing path {:?}", path);

    // initialize the audio stuffs
    let event_loop = cpal::EventLoop::new();

    let input_device = cpal::default_input_device().expect("Failed to get default input device");
    let output_device = cpal::default_output_device().expect("Failed to get default output device");
    println!("Using default input device: \"{}\"", input_device.name());
    println!("Using default output device: \"{}\"", output_device.name());

    // We'll try and use the same format between streams to keep it simple
    let mut format = input_device.default_output_format().expect("Failed to get default format");
    format.data_type = cpal::SampleFormat::F32;

    // Build streams.
    println!("Attempting to build both streams with `{:?}`.", format);
    let input_stream_id = event_loop.build_input_stream(&input_device, &format).unwrap();
    let output_stream_id = event_loop.build_output_stream(&output_device, &format).unwrap();
    println!("Successfully built streams.");

    event_loop.play_stream(output_stream_id.clone());

    let mut file = File::open(path).unwrap();

    let mut finished = false;
    event_loop.run(move |id, data| {
        if finished {
            exit(0);
        }
        match data {
            cpal::StreamData::Output { buffer: cpal::UnknownTypeOutputBuffer::F32(mut buffer) } => {
                assert_eq!(id, output_stream_id);
                for sample in buffer.iter_mut() {
                    *sample = match file.read_f32::<BigEndian>() {
                        Ok(s) => s,
                        Err(err) => {
                            finished = true;
                            0.0
                        }
                    };
                }
            }
            _ => () // panic!("we're expecting f32 data"),
        }
    });
}