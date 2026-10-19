# WebSocket conformance

Run on Linux with Cargo and Docker available:

```sh
ci/autobahn/run.sh
ci/autobahn/run.sh autobahn-echo-compio
AUTOBAHN_CASES='1.*,2.*' ci/autobahn/run.sh
```

The first command tests Tokio; the second tests Compio. `AUTOBAHN_CASES`
replaces the selected cases in `fuzzingclient.json`. The configured suite
excludes large-message/performance groups 9 and compression groups 12 and 13.

Exit status 0 means every selected case is OK or NON-STRICT. Status 1 means
the runner or server failed, a report is missing, or a case reports FAILED,
INFORMATIONAL_FAILED, WRONG_CODE, or UNCLEAN. Reports are written under
`ci/autobahn/reports/`.
