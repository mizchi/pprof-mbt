// k6 load test for the moonbit http_server_benchmark.
// Runs 64 / 128 / 256 VUs for 15s each, matching the wrk pattern in
// moonbitlang/async's bench.sh.

import http from 'k6/http';

export const options = {
  scenarios: {
    constant: {
      executor: 'constant-vus',
      vus: __ENV.VUS ? parseInt(__ENV.VUS) : 64,
      duration: __ENV.DURATION || '15s',
    },
  },
};

export default function () {
  http.get('http://127.0.0.1:30001/');
}
