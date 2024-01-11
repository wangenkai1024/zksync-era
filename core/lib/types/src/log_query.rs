use zk_evm::{
    aux_structures::Timestamp as Timestamp_1_4_0,
    zk_evm_abstractions::queries::LogQuery as LogQuery_1_4_0,
};
use zk_evm_1_4_1::{
    aux_structures::Timestamp as Timestamp_1_4_1,
    zk_evm_abstractions::queries::LogQuery as LogQuery_1_4_1,
};
use zksync_basic_types::{Address, U256};

/// Struct representing the VM timestamp
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Timestamp(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LogQuery {
    pub timestamp: Timestamp,
    pub tx_number_in_block: u16,
    pub aux_byte: u8,
    pub shard_id: u8,
    pub address: Address,
    pub key: U256,
    pub read_value: U256,
    pub written_value: U256,
    pub rw_flag: bool,
    pub rollback: bool,
    pub is_service: bool,
}

impl From<LogQuery_1_4_0> for LogQuery {
    fn from(value: LogQuery_1_4_0) -> Self {
        Self {
            timestamp: Timestamp(value.timestamp.0),
            tx_number_in_block: value.tx_number_in_block,
            aux_byte: value.aux_byte,
            shard_id: value.shard_id,
            address: value.address,
            key: value.key,
            read_value: value.read_value,
            written_value: value.written_value,
            rw_flag: value.rw_flag,
            rollback: value.rollback,
            is_service: value.is_service,
        }
    }
}

impl From<LogQuery_1_4_1> for LogQuery {
    fn from(value: LogQuery_1_4_1) -> Self {
        Self {
            timestamp: Timestamp(value.timestamp.0),
            tx_number_in_block: value.tx_number_in_block,
            aux_byte: value.aux_byte,
            shard_id: value.shard_id,
            address: value.address,
            key: value.key,
            read_value: value.read_value,
            written_value: value.written_value,
            rw_flag: value.rw_flag,
            rollback: value.rollback,
            is_service: value.is_service,
        }
    }
}

impl Into<LogQuery_1_4_0> for LogQuery {
    fn into(self) -> LogQuery_1_4_0 {
        LogQuery_1_4_0 {
            timestamp: Timestamp_1_4_0(self.timestamp.0),
            tx_number_in_block: self.tx_number_in_block,
            aux_byte: self.aux_byte,
            shard_id: self.shard_id,
            address: self.address,
            key: self.key,
            read_value: self.read_value,
            written_value: self.written_value,
            rw_flag: self.rw_flag,
            rollback: self.rollback,
            is_service: self.is_service,
        }
    }
}

impl Into<LogQuery_1_4_1> for LogQuery {
    fn into(self) -> LogQuery_1_4_1 {
        LogQuery_1_4_1 {
            timestamp: Timestamp_1_4_1(self.timestamp.0),
            tx_number_in_block: self.tx_number_in_block,
            aux_byte: self.aux_byte,
            shard_id: self.shard_id,
            address: self.address,
            key: self.key,
            read_value: self.read_value,
            written_value: self.written_value,
            rw_flag: self.rw_flag,
            rollback: self.rollback,
            is_service: self.is_service,
        }
    }
}