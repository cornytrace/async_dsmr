use std::io::ErrorKind;

use crc::{Crc, Digest, CRC_16_ARC};
use tokio_util::bytes::{Buf, BytesMut};

type DataObject = String;

#[derive(Clone, Debug)]
pub struct Telegram {
    pub manufacturer: [u8; 3],
    pub ident: String,
    pub data: Vec<DataObject>,
}

enum FrameState {
    Empty,
    PartialData(Telegram),
}

const CRC: Crc<u16> = Crc::<u16>::new(&CRC_16_ARC);

pub struct ModeDFrame<'a> {
    state: FrameState,
    crc: Digest<'a, u16>,
}

impl<'a> ModeDFrame<'a> {
    pub fn new() -> ModeDFrame<'a> {
        ModeDFrame {
            state: FrameState::Empty,
            crc: CRC.digest(),
        }
    }

    pub fn reset(&mut self) {
        self.state = FrameState::Empty;
        self.crc = CRC.digest();
    }
}

impl<'a> Default for ModeDFrame<'a> {
    fn default() -> Self {
        Self::new()
    }
}

// const MAX_LENGTH: usize = 5 + 16 + 4 + 1024 + 1 + 4 + 2;

impl<'a> tokio_util::codec::Decoder for ModeDFrame<'a> {
    type Item = Telegram;
    type Error = std::io::Error;

    fn decode(&mut self, bytes: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        loop {
            match &mut self.state {
                FrameState::Empty => {
                    match bytes.iter().position(|c| *c == b'/') {
                        Some(x) if x > 0 => {
                            bytes.advance(x);
                            // Reset CRC
                            self.crc = CRC.digest();
                        }
                        Some(0) => {}
                        _ => return Ok(None),
                    }
                    match bytes.iter().position(|c| *c == b'\n') {
                        Some(end) => {
                            let line = bytes.split_to(end + 3);
                            self.crc.update(&line);
                            if line.len() < 6 {
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidInput,
                                    "header too short",
                                ));
                            }
                            let line = line.to_vec();
                            self.state = FrameState::PartialData(Telegram {
                                manufacturer: line[1..4].try_into().unwrap(),
                                ident: String::from_utf8(line[5..].to_vec())
                                    .map_err(|_| {
                                        std::io::Error::new(
                                            std::io::ErrorKind::InvalidData,
                                            "identifier not valid UTF-8",
                                        )
                                    })?
                                    .trim()
                                    .to_string(),
                                data: Vec::new(),
                            });
                        }
                        None => return Ok(None),
                    }
                }
                FrameState::PartialData(t) => {
                    loop {
                        match bytes.iter().position(|c| *c == b'\n') {
                            Some(end) => {
                                let line_bytes = bytes.split_to(end + 1);
                                let line =
                                    String::from_utf8(line_bytes.to_vec()).map_err(|_| {
                                        std::io::Error::new(
                                            std::io::ErrorKind::InvalidData,
                                            format!("data line {} not valid UTF-8", t.data.len()),
                                        )
                                    })?;
                                if line.contains('!') {
                                    if line.starts_with('!') {
                                        self.crc.update(&[b'!']);
                                        let comp_crc = self.crc.clone().finalize();
                                        let recv_crc =
                                            u16::from_str_radix(&line[1..5], 16).unwrap();
                                        if comp_crc != recv_crc {
                                            return Err(std::io::Error::new(ErrorKind::InvalidData, format!("CRC error! computed: {:#04X}, received: {:#04X}", comp_crc, recv_crc)));
                                        }
                                        let telegram = t.clone();
                                        self.reset();
                                        return Ok(Some(telegram));
                                    } else {
                                        return Err(std::io::Error::new(ErrorKind::InvalidData, "Invalid exclamation point detected: not at start of line"));
                                    }
                                }
                                self.crc.update(&line_bytes);
                                let line = line.trim().to_string();

                                t.data.push(line);
                            }
                            None => return Ok(None),
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::fs::File;
    use tokio_util::codec::Framed;

    use futures_util::StreamExt;

    use std::error::Error;

    use crate::*;

    #[tokio::test]
    async fn test_decoder() -> Result<(), Box<dyn Error>> {
        let file = File::open("test/capture.txt").await?;
        let mut frames = Framed::new(file, ModeDFrame::new());
        while let Some(t) = frames.next().await {
            println!("{:?}", t?)
        }
        Ok(())
    }
}
