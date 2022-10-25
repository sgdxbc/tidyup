use messages::OpNumber;

pub trait App {
    fn execute(&mut self, op_number: OpNumber, op: &[u8]) -> Box<[u8]>;
}

pub struct Null;
impl App for Null {
    fn execute(&mut self, _n: OpNumber, _op: &[u8]) -> Box<[u8]> {
        Box::new([])
    }
}
