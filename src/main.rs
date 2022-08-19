use std::collections::BTreeMap;

use anyhow::Result;
use rav1e::prelude::*;

use hdr10plus::metadata::Hdr10PlusMetadata;
use hdr10plus::metadata_json::MetadataJsonRoot;

fn main() -> Result<()> {
    let (width, height) = (640, 480);
    let enc = EncoderConfig {
        width,
        height,
        speed_settings: SpeedSettings::from_preset(10),
        chroma_sample_position: ChromaSamplePosition::Colocated,
        color_description: Some(ColorDescription {
            color_primaries: ColorPrimaries::BT2020,
            transfer_characteristics: TransferCharacteristics::SMPTE2084,
            matrix_coefficients: MatrixCoefficients::BT2020NCL,
        }),
        pixel_range: PixelRange::Limited,
        ..Default::default()
    };

    let cfg = Config::new().with_encoder_config(enc.clone());
    let mut ctx: Context<u16> = cfg.new_context()?;

    let mut out = std::fs::File::create("hdr10plus_av1.ivf")?;
    ivf::write_ivf_header(&mut out, width, height, 24000, 1001);

    let mut f = ctx.new_frame();

    let pixels = vec![128; enc.width * enc.height];

    for p in &mut f.planes {
        let stride = (enc.width + p.cfg.xdec) >> p.cfg.xdec;
        p.copy_from_raw_u8(&pixels, stride, 1);
    }

    let mut meta_frames: BTreeMap<usize, Vec<u8>> = MetadataJsonRoot::from_file("metadata.json")?
        .scene_info
        .iter()
        .filter_map(|meta| {
            Hdr10PlusMetadata::try_from(meta)
                .and_then(|meta| meta.encode(true))
                .ok()
                .map(|mut bytes| {
                    bytes.remove(0);
                    bytes
                })
        })
        .enumerate()
        .collect();

    let limit = meta_frames.len();
    let frame_type_override = FrameTypeOverride::No;

    for i in 0..limit {
        println!("Sending frame {}", i);

        let t35_metadata = if let Some(meta) = meta_frames.remove(&i) {
            Box::new([T35 {
                country_code: 0xB5,
                country_code_extension_byte: 0x00,
                data: meta.into_boxed_slice(),
            }])
        } else {
            Box::default()
        };

        let fp = FrameParameters {
            frame_type_override,
            opaque: None,
            t35_metadata,
        };

        match ctx.send_frame((f.clone(), fp)) {
            Ok(_) => {}
            Err(e) => match e {
                EncoderStatus::EnoughData => {
                    println!("Unable to append frame {} to the internal queue", i);
                }
                _ => {
                    panic!("Unable to send frame {}", i);
                }
            },
        }
    }

    ctx.flush();

    // Test that we cleanly exit once we hit the limit
    let mut i = 0;
    while i < limit + 5 {
        match ctx.receive_packet() {
            Ok(pkt) => {
                println!("Packet {}", pkt.input_frameno);
                i += 1;

                ivf::write_ivf_frame(&mut out, pkt.input_frameno, &pkt.data);
            }
            Err(e) => match e {
                EncoderStatus::LimitReached => {
                    println!("Limit reached");
                    break;
                }
                EncoderStatus::Encoded => println!("  Encoded"),
                EncoderStatus::NeedMoreData => println!("  Need more data"),
                _ => {
                    panic!("Unable to receive packet {}", i);
                }
            },
        }
    }

    Ok(())
}
