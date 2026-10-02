import assert from "node:assert/strict";
import { test } from "node:test";

import { OracleContractError, OracleError, parseOracleError } from "./client.js";

test("parses contract error codes from simulation diagnostics", () => {
  const message =
    'HostError: Error(Contract, #8)\n\nEvent log (newest first):\n   0: [Diagnostic Event] ...';
  assert.equal(parseOracleError(message), OracleError.Stale);
});

test("ignores host errors that are not contract errors", () => {
  assert.equal(parseOracleError("HostError: Error(Storage, MissingValue)"), undefined);
});

test("names the error in the exception message", () => {
  const err = new OracleContractError(OracleError.NoData);
  assert.equal(err.message, "gas oracle error 7 (NoData)");
});
