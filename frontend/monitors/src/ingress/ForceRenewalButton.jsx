import { useState } from 'react';

/**
 * ForceRenewalButton
 *
 * Triggers a cert-manager annotation patch to force certificate renewal.
 * Shows a confirmation step before firing to prevent accidental clicks.
 * Calls the optional `onRenew` callback with the certRow when confirmed.
 *
 * Props:
 *   certRow         – CertRow object (used for the confirmation label + callback)
 *   onRenew         – async function(certRow) → void | Promise<void>
 *                     The parent is responsible for the actual API call / patch.
 *   disabled        – (boolean) hard-disable the button (e.g. cert is not cert-manager managed)
 */
export default function ForceRenewalButton({ certRow, onRenew, disabled = false }) {
  const [phase, setPhase] = useState('idle'); // 'idle' | 'confirm' | 'loading' | 'done' | 'error'
  const [errorMsg, setErrorMsg] = useState('');

  if (!certRow.certManagerManaged && !certRow.renewalTriggered) {
    return (
      <span className="renewal-btn renewal-btn--unmanaged" title="Not managed by cert-manager">
        Manual
      </span>
    );
  }

  if (certRow.renewalTriggered || phase === 'done') {
    return (
      <span className="renewal-btn renewal-btn--triggered" aria-live="polite">
        ✓ Renewal queued
      </span>
    );
  }

  if (phase === 'loading') {
    return (
      <span className="renewal-btn renewal-btn--loading" aria-live="polite" aria-busy="true">
        <span className="renewal-spinner" aria-hidden="true" /> Renewing…
      </span>
    );
  }

  if (phase === 'error') {
    return (
      <button
        type="button"
        className="renewal-btn renewal-btn--error"
        onClick={() => setPhase('idle')}
        aria-label={`Renewal failed for ${certRow.host}. Click to retry.`}
      >
        ✗ Retry
      </button>
    );
  }

  if (phase === 'confirm') {
    return (
      <span className="renewal-btn-confirm" role="group" aria-label="Confirm renewal">
        <span className="renewal-confirm-text">Renew now?</span>
        <button
          type="button"
          className="renewal-btn renewal-btn--yes"
          aria-label={`Confirm force renewal for ${certRow.host}`}
          onClick={async () => {
            setPhase('loading');
            setErrorMsg('');
            try {
              await onRenew?.(certRow);
              setPhase('done');
            } catch (err) {
              setErrorMsg(err?.message ?? 'Renewal failed');
              setPhase('error');
            }
          }}
        >
          Yes
        </button>
        <button
          type="button"
          className="renewal-btn renewal-btn--cancel"
          aria-label="Cancel renewal"
          onClick={() => setPhase('idle')}
        >
          Cancel
        </button>
      </span>
    );
  }

  // idle
  return (
    <button
      type="button"
      className="renewal-btn renewal-btn--idle"
      disabled={disabled}
      aria-label={`Force renewal for ${certRow.host}`}
      onClick={() => setPhase('confirm')}
    >
      Force Renewal
    </button>
  );
}
