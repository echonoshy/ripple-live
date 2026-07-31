use std::io::Cursor;

use anyhow::bail;

pub fn float32_to_wav(samples: &[f32], sample_rate: u32) -> anyhow::Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
        for sample in samples {
            let pcm = (sample.clamp(-1.0, 1.0) * 32_767.0) as i16;
            writer.write_sample(pcm)?;
        }
        writer.finalize()?;
    }
    Ok(cursor.into_inner())
}

pub fn decode_le_f32(payload: &[u8]) -> anyhow::Result<Vec<f32>> {
    if !payload.len().is_multiple_of(4) {
        bail!("audio payload length is not aligned to float32");
    }
    Ok(payload
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four-byte chunk")))
        .collect())
}

pub fn encode_le_f32(samples: &[f32]) -> Vec<u8> {
    let mut output = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        output.extend_from_slice(&sample.to_le_bytes());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_encoding_is_valid() {
        let input = vec![0.0, 0.5, -0.5, 0.25];
        let wav = float32_to_wav(&input, 16_000).unwrap();
        let reader = hound::WavReader::new(Cursor::new(wav)).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.len(), input.len() as u32);
    }
}
