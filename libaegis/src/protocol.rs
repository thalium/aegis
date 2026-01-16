/// The protocol used for communicating between the host and the VM
/// TODO: add detail

/// Initialize a connection
pub const INIT_MSG: &str = "HELLO";

/// The VM can continue its operations
pub const CONTINUE_MSG: &str = "CONTINUE";

/// Ask the host to read data
pub const READ_MSG: &str = "READ";

/// Ask the host to write data
pub const WRITE_MSG: &str = "WRITE";

/// Ask the connection to be closed
pub const EXIT_MSG: &str = "EXIT";

pub const WRITE_REGION_OFFSET: usize = 0x_0080_0000;
