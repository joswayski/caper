mod media;
mod media_gateway;
mod state;

use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use libwebrtc::peer_connection_factory::{PeerConnectionFactory, RtcConfiguration};

fn main() -> Result<(), String> {
    let factory = PeerConnectionFactory::default();
    let acquired = factory.acquire_platform_adm();
    if !acquired {
        return Err("native WebRTC audio device module is unavailable".into());
    }

    let inputs = factory.recording_devices().max(0) as u16;
    let outputs = factory.playout_devices().max(0) as u16;
    println!("native ADM ready: {inputs} recording device(s), {outputs} playout device(s)");
    for index in 0..inputs {
        println!("input {index}: {}", factory.recording_device_name(index));
    }
    for index in 0..outputs {
        println!("output {index}: {}", factory.playout_device_name(index));
    }

    // This creates and links the same raw native PeerConnection used by the
    // Caper engine. It intentionally does not contact a signaling endpoint.
    let peer = factory
        .create_peer_connection(RtcConfiguration::default())
        .map_err(|error| error.to_string())?;
    peer.close();
    factory.release_platform_adm();
    Ok(())
}
