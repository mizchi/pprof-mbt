///|
/// regex.execute benchmark for profiling moonbitlang/core.
///
/// Mirrors core's "bench regex phone lines" case — a moderately complex
/// pattern run across a 34-line corpus. We loop the corpus enough times
/// to give the sampler a good signal.

///|
let phone_lines : Array[String] = [
  "datafile for regex matching of phone numbers test", "",
  "the following numbers should match:", "", "(111) 111 1111", "111 222-2222",
  "(111) 333-3333", "111 444 4444", "", "foo (111) 555 5555",
  "foo 111 666-6666", "foo (111) 777-7777", "foo 111 888 8888", "",
  "foo (111) 999 9999 bar", "foo 111 000-0000 bar", "foo (111) 232-1111 bar",
  "foo 111 242 1111 bar", "", "The following should fail to match:", "",
  "2(212) 222 2222", "2232 222-2222", "(242) 222-22222", "2252 222 2222", "",
  "foo (262) 2222222", "foo 272 2222222", "foo (282) 222-22227",
  "foo 292 222 22222", "", "foo (202).222 2222 bar", "foo 211@22222222 bar",
  "foo (213 222-2222 bar", "foo 214) 222 2222 bar",
]

///|
fn main {
  let re = re"([(][0-9]{3}[)]|[0-9]{3}) [0-9]{3}[ \-][0-9]{4}"
  let iters = 2000
  let mut matches = 0
  for _ in 0..<iters {
    for line in phone_lines {
      match re.execute(line) {
        Some(_) => matches = matches + 1
        None => ()
      }
    }
  }
  println("matches=\{matches} iters=\{iters} corpus_lines=\{phone_lines.length()}")
}
