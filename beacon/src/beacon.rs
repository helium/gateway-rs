use crate::Entropy;
use helium_proto::{
    services::poc_lora,
    {BlockchainRegionParamV1, DataRate},
};
use rand::{seq::SliceRandom, Rng, SeedableRng};
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const BEACON_PAYLOAD_SIZE: usize = 10;

#[derive(Debug, Clone)]
pub struct Beacon {
    pub data: Vec<u8>,

    pub frequency: u64,
    pub datarate: DataRate,
    pub remote_entropy: Entropy,
    pub local_entropy: Entropy,
}

impl Beacon {
    pub fn new(
        remote_entropy: Entropy,
        local_entropy: Entropy,
        region_params: &[BlockchainRegionParamV1],
    ) -> Self {
        let data = {
            let mut hasher = Sha256::new();
            remote_entropy.digest(&mut hasher);
            local_entropy.digest(&mut hasher);
            hasher.finalize().to_vec()
        };
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&data[0..32]);
        let mut rng = rand_chacha::ChaCha12Rng::from_seed(seed);

        let frequency = rand_frequency(region_params, &mut rng);
        let datarate = DataRate::Sf7bw125;

        Self {
            data: {
                data.truncate(BEACON_PAYLOAD_SIZE);
                data
            },
            frequency,
            datarate,
            local_entropy,
            remote_entropy,
        }
    }

    pub fn beacon_id(&self) -> String {
        base64::encode(&self.data)
    }
}

fn rand_frequency<R>(region_params: &[BlockchainRegionParamV1], rng: &mut R) -> u64
where
    R: Rng + ?Sized,
{
    assert!(!region_params.is_empty());
    region_params
        .choose(rng)
        .map(|params| params.channel_frequency)
        .unwrap()
}

impl From<Beacon> for poc_lora::LoraBeaconReportReqV1 {
    fn from(v: Beacon) -> Self {
        Self {
            pub_key: vec![],
            local_entropy: v.local_entropy.data,
            remote_entropy: v.remote_entropy.data,
            data: v.data,
            frequency: v.frequency,
            channel: 0,
            datarate: v.datarate as i32,
            tx_power: 27,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_else(|_| Duration::from_secs(0))
                .as_nanos() as u64,
            signature: vec![],
        }
    }
}
