pub enum App {
    Null,
    // YCSB
}

impl App {
    pub fn execute(&mut self, _op_number: u32, op: &[u8]) -> Box<[u8]> {
        match self {
            Self::Null => {
                let _ = op;
                Default::default()
            }
        }
    }
}
