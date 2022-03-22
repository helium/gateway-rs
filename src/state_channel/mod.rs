mod channel;
mod message;

pub use channel::{
    check_active, check_active_diff, StateChannel, StateChannelCausality, StateChannelValidation,
};
pub use message::StateChannelMessage;
