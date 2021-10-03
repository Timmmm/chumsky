use super::*;

/// A trait implemented by error recovery strategies.
pub trait Strategy<I: Clone, O> {
    /// Recover from a parsing failure.
    fn recover<P: Parser<I, O>>(
        &self,
        recovered_errors: Vec<Located<I, P::Error>>,
        fatal_error: Located<I, P::Error>,
        parser: P,
        stream: &mut StreamOf<I, P::Error>,
    ) -> PResult<I, O, P::Error>;
}

/// See [`skip_then_retry_until`].
#[derive(Copy, Clone)]
pub struct SkipThenRetryUntil<I, const N: usize>(pub [I; N]);

impl<I: Clone + PartialEq, O, const N: usize> Strategy<I, O> for SkipThenRetryUntil<I, N> {
    fn recover<P: Parser<I, O>>(
        &self,
        a_errors: Vec<Located<I, P::Error>>,
        a_err: Located<I, P::Error>,
        parser: P,
        stream: &mut StreamOf<I, P::Error>,
    ) -> PResult<I, O, P::Error> {
        loop {
            if !stream.attempt(|stream| if stream.next().2.map_or(true, |tok| self.0.contains(&tok)) {
                (false, false)
            } else {
                (true, true)
            }) {
                break (a_errors, Err(a_err));
            }
            #[allow(deprecated)]
            let (mut errors, res) = parser.parse_inner(stream);
            if let Ok(out) = res {
                errors.push(a_err);
                break (errors, Ok(out));
            }
        }
    }
}

/// A recovery mode that simply skips to the next input on parser failure and tries again, until reaching on of a
/// series of inputs.
///
/// This strategy is very 'stupid' and can result in very poor error generation in some languages. Place this strategy
/// after others as a last resort, and be careful about over-using it.
pub fn skip_then_retry_until<I, const N: usize>(until: [I; N]) -> SkipThenRetryUntil<I, N> {
    SkipThenRetryUntil(until)
}

/// See [`nested_delimiters`].
#[derive(Copy, Clone)]
pub struct NestedDelimiters<I, F, const N: usize>(pub I, pub I, pub [(I, I); N], pub F);

impl<I: Clone + PartialEq, O, F: Fn() -> O, const N: usize> Strategy<I, O> for NestedDelimiters<I, F, N> {
    fn recover<P: Parser<I, O>>(
        &self,
        mut a_errors: Vec<Located<I, P::Error>>,
        a_err: Located<I, P::Error>,
        _parser: P,
        stream: &mut StreamOf<I, P::Error>,
    ) -> PResult<I, O, P::Error> {
        assert!(self.0 != self.1, "NestedDelimiters cannot be used with identical delimiters.");

        let mut balance = 0;
        let mut balance_others = [0; N];
        let mut starts = Vec::new();
        let mut error = None;
        let recovered = loop {
            // let pre_state = stream.save();
            if match stream.next() {
                (_, span, Some(t)) if t == self.0 => { balance += 1; starts.push(span); true },
                (_, _, Some(t)) if t == self.1 => { balance -= 1; starts.pop(); true },
                (at, span, Some(t)) => {
                    for i in 0..N {
                        if t == self.2[i].0 {
                            balance_others[i] += 1;
                        } else if t == self.2[i].1 {
                            balance_others[i] -= 1;

                            if balance_others[i] < 0 && balance == 1 {
                                // stream.revert(pre_state);
                                error.get_or_insert_with(|| Located::at(at, P::Error::unclosed_delimiter(starts.pop().unwrap(), self.0.clone(), span.clone(), self.1.clone(), Some(t.clone()))));
                            }
                        }
                    }
                    false
                },
                (at, span, None) => {
                    if balance > 0 && balance == 1 {
                        error.get_or_insert_with(|| match starts.pop() {
                            Some(start) => Located::at(at, P::Error::unclosed_delimiter(start, self.0.clone(), span, self.1.clone(), None)),
                            None => Located::at(at, P::Error::expected_input_found(span, Some(self.1.clone()), None)),
                        });
                    }
                    break false
                },
            } {
                if balance == 0 {
                    break true;
                } else if balance < 0 {
                    // The end of a delimited section is not a valid recovery pattern
                    break false;
                }
            } else if balance == 0 {
                // A non-delimiter input before anything else is not a valid recovery pattern
                break false;
            }
        };

        if let Some(e) = error { a_errors.push(e); }

        if recovered {
            if a_errors.last().map_or(true, |e| a_err.at < e.at) {
                a_errors.push(a_err);
            }
            (a_errors, Ok(((self.3)(), None)))
        } else {
            (a_errors, Err(a_err))
        }
    }
}

/// A recovery strategy that searches for a start and end delimiter, respecting nesting.
///
/// It is possible to specify other delimiters that are valid in this scope for better error generation. A function
/// that generates a default fallback parser output on recovery is also required.
pub fn nested_delimiters<I, F, const N: usize>(start: I, end: I, others: [(I, I); N], default: F) -> NestedDelimiters<I, F, N> {
    NestedDelimiters(start, end, others, default)
}

/// A parser that includes a fallback recovery strategy should parsing result in an error.
#[derive(Copy, Clone)]
pub struct Recovery<A, S>(pub(crate) A, pub(crate) S);

impl<I: Clone, O, A: Parser<I, O, Error = E>, S: Strategy<I, O>, E: Error<I>> Parser<I, O> for Recovery<A, S> {
    type Error = E;

    fn parse_inner(&self, stream: &mut StreamOf<I, Self::Error>) -> PResult<I, O, Self::Error> {
        match stream.try_parse(|stream| { #[allow(deprecated)] self.0.parse_inner(stream) }) {
            (a_errors, Ok(a_out)) => (a_errors, Ok(a_out)),
            (a_errors, Err(a_err)) => self.1.recover(a_errors, a_err, &self.0, stream),
        }
    }
}
