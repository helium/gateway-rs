use crate::{Entropy, Error, Result};
use helium_proto::{
    services::poc_lora,
    {BlockchainRegionParamV1, DataRate},
};
use rand::{seq::SliceRandom, Rng, SeedableRng};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_BEACON_V0_PAYLOAD_SIZE: usize = 10;
pub const MIN_BEACON_V0_PAYLOAD_SIZE: usize = 5;

#[derive(Debug, Clone)]
pub struct Beacon {
    pub data: Vec<u8>,

    pub frequency: u64,
    pub datarate: DataRate,
    pub remote_entropy: Entropy,
    pub local_entropy: Entropy,
}

impl Beacon {
    /// Construct a new beacon with a given remote and local entropy. The remote
    /// and local entropy are checked for version equality.
    ///
    /// Version 0 beacons use a Sha256 of the remote and local entropy (data and
    /// timestamp), which is then used as a 32 byte seed to a ChaCha12 rng. This
    /// rng is used to choose a random frequency from the given region
    /// parameters and a payload size between MIN_BEACON_V0_PAYLOAD_SIZE and
    /// MAX_BEACON_V0_PAYLOAD_SIZE.
    pub fn new(
        remote_entropy: Entropy,
        local_entropy: Entropy,
        region_params: &[BlockchainRegionParamV1],
    ) -> Result<Self> {
        match remote_entropy.version {
            0 => {
                let mut data = {
                    let mut hasher = Sha256::new();
                    remote_entropy.digest(&mut hasher);
                    local_entropy.digest(&mut hasher);
                    hasher.finalize().to_vec()
                };
                let mut seed = [0u8; 32];
                seed.copy_from_slice(&data[0..32]);
                let mut rng = rand_chacha::ChaCha12Rng::from_seed(seed);

                let frequency = rand_frequency(region_params, &mut rng)?;
                let payload_size =
                    rng.gen_range(MIN_BEACON_V0_PAYLOAD_SIZE..=MAX_BEACON_V0_PAYLOAD_SIZE);
                let datarate = DataRate::Sf7bw125;

                Ok(Self {
                    data: {
                        data.truncate(payload_size);
                        data
                    },
                    frequency,
                    datarate,
                    local_entropy,
                    remote_entropy,
                })
            }
            _ => Err(Error::invalid_version()),
        }
    }

    pub fn beacon_id(&self) -> String {
        base64::encode(&self.data)
    }
}

fn rand_frequency<R>(region_params: &[BlockchainRegionParamV1], rng: &mut R) -> Result<u64>
where
    R: Rng + ?Sized,
{
    region_params
        .choose(rng)
        .map(|params| params.channel_frequency)
        .ok_or_else(Error::no_region_params)
}

impl TryFrom<Beacon> for poc_lora::LoraBeaconReportReqV1 {
    type Error = Error;
    fn try_from(v: Beacon) -> Result<Self> {
        Ok(Self {
            pub_key: vec![],
            local_entropy: v.local_entropy.data,
            remote_entropy: v.remote_entropy.data,
            data: v.data,
            frequency: v.frequency,
            channel: 0,
            datarate: v.datarate as i32,
            tmst: 0,
            tx_power: 27,
            // The timestamp of the beacon is the timestamp of creation of the
            // report (in nanos)
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(Error::from)?
                .as_nanos() as u64,
            signature: vec![],
        })
    }
}
