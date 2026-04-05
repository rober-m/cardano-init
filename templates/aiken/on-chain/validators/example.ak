/// A simple always-succeeds validator.
/// Replace this with your own validator logic.
validator example {
  spend(_datum: Option<Data>, _redeemer: Data, _output_ref: Data, _tx: Data) {
    True
  }
}
