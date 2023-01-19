use crate::{Entropy, Error, RegionParams, Result};
use helium_proto::{services::poc_iot, DataRate};
use rand::{seq::SliceRandom, Rng, SeedableRng};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_BEACON_V0_PAYLOAD_SIZE: usize = 10;
pub const MIN_BEACON_V0_PAYLOAD_SIZE: usize = 5;

// Supported datarates worldwide. Note that SF12 is not supported everywhere
pub const BEACON_DATA_RATES: &[DataRate] = &[
    DataRate::Sf7bw125,
    DataRate::Sf8bw125,
    DataRate::Sf9bw125,
    DataRate::Sf10bw125,
];

#[derive(Debug, Clone)]
pub struct Beacon {
    pub data: Vec<u8>,

    pub frequency: u64,
    pub datarate: DataRate,
    pub remote_entropy: Entropy,
    pub local_entropy: Entropy,
    pub conducted_power: u32,
}

impl Beacon {
    /// Construct a new beacon with a given remote and local entropy. The remote
    /// and local entropy are checked for version equality.
    ///
    /// Version 0 beacons use a Sha256 of the remote and local entropy (data and
    /// timestamp), which is then used as a 32 byte seed to a ChaCha12 rng. This
    /// rng is used to choose a random data rate, a random frequency and
    /// conducted power from the given region parameters and a payload size
    /// between MIN_BEACON_V0_PAYLOAD_SIZE and MAX_BEACON_V0_PAYLOAD_SIZE.
    pub fn new(
        remote_entropy: Entropy,
        local_entropy: Entropy,
        region_params: &RegionParams,
    ) -> Result<Self> {
        match remote_entropy.version {
            0 | 1 => {
                let mut data = {
                    let mut hasher = Sha256::new();
                    remote_entropy.digest(&mut hasher);
                    local_entropy.digest(&mut hasher);
                    hasher.finalize().to_vec()
                };

                // Construct a 32 byte seed from the hash of the local and
                // remote entropy
                let mut seed = [0u8; 32];
                seed.copy_from_slice(&data[0..32]);
                // Make a random generator
                let mut rng = rand_chacha::ChaCha12Rng::from_seed(seed);

                // Note that the ordering matters since the random number
                // generator is used in this order.
                let frequency = region_params.rand_frequency(&mut rng)?;
                let payload_size =
                    rng.gen_range(MIN_BEACON_V0_PAYLOAD_SIZE..=MAX_BEACON_V0_PAYLOAD_SIZE);

                let datarate = rand_data_rate(BEACON_DATA_RATES, &mut rng)?;
                let conducted_power = region_params.rand_conducted_power(&mut rng)?;

                Ok(Self {
                    data: {
                        data.truncate(payload_size);
                        data
                    },
                    frequency,
                    datarate: datarate.to_owned(),
                    local_entropy,
                    remote_entropy,
                    conducted_power,
                })
            }
            _ => Err(Error::invalid_version()),
        }
    }

    pub fn beacon_id(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&self.data)
    }
}

fn rand_data_rate<'a, R>(data_rates: &'a [DataRate], rng: &mut R) -> Result<&'a DataRate>
where
    R: Rng + ?Sized,
{
    data_rates.choose(rng).ok_or_else(Error::no_data_rate)
}

impl TryFrom<Beacon> for poc_iot::IotBeaconReportReqV1 {
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
            // This is the initial value. The beacon sender updates this value
            // with the actual conducted power reported by the packet forwarder
            tx_power: v.conducted_power as i32,
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
