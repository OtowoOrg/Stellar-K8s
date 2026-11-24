/**
 * JSONModal.tsx
 *
 * Full-screen overlay that pretty-prints the XDR-decoded payload for a
 * selected Soroban contract event.
 *
 * Keyboard: Escape closes the modal.
 * Accessibility: role="dialog", aria-modal, focus trap on open.
 */

import React, { useEffect, useRef, useCallback } from 'react';
import type { RawContractEvent } from '../../services/event_stream';
import { decodeEventPayload, prettyPrintScVal } from './xdr_decoder';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface JSONModalProps {
  event: RawContractEvent | null;
  onClose: () => void;
}

// ---------------------------------------------------------------------------
// Syntax highlighter — minimal, zero-dependency
// ---------------------------------------------------------------------------

function highlight(json: string): string {
  return json
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(
      /("(\\u[a-zA-Z0-9]{4}|\\[^u]|[^\\"])*"(\s*:)?|\b(true|false|null)\b|-?\d+(?:\.\d*)?(?:[eE][+\-]?\d+)?)/g,
      (match) => {
        let cls = 'json-number';
        if (/^"/.test(match)) {
          cls = /:$/.test(match) ? 'json-key' : 'json-string';
        } else if (/true|false/.test(match)) {
          cls = 'json-bool';
        } else if (/null/.test(match)) {
          cls = 'json-null';
        }
        return `<span class="${cls}">${match}</span>`;
      },
    );
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function JSONModal({ event, onClose }: JSONModalProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);

  // Decode payload when event changes.
  const decoded = event ? decodeEventPayload(event.topics, event.value_xdr) : null;

  const topicsJson = decoded
    ? decoded.topics.map((t) => prettyPrintScVal(t)).join('\n')
    : '';
  const valueJson = decoded ? prettyPrintScVal(decoded.value) : '';

  // Focus the close button on open; restore previous focus on close.
  const prevFocusRef = useRef<Element | null>(null);
  useEffect(() => {
    if (event) {
      prevFocusRef.current = document.activeElement;
      // Small delay to let React render the dialog first.
      requestAnimationFrame(() => closeButtonRef.current?.focus());
    } else if (prevFocusRef.current instanceof HTMLElement) {
      prevFocusRef.current.focus();
    }
  }, [event]);

  // Close on Escape.
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    },
    [onClose],
  );

  // Trap focus inside the dialog.
  const handleFocusTrap = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key !== 'Tab') return;
      const dialog = dialogRef.current;
      if (!dialog) return;
      const focusable = dialog.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      );
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (e.shiftKey) {
        if (document.activeElement === first) {
          e.preventDefault();
          last?.focus();
        }
      } else {
        if (document.activeElement === last) {
          e.preventDefault();
          first?.focus();
        }
      }
    },
    [],
  );

  const copyToClipboard = useCallback(
    (text: string, label: string) => {
      navigator.clipboard
        .writeText(text)
        .then(() => {
          // Brief visual feedback via aria-live region (handled by parent).
        })
        .catch(() => {/* clipboard permission denied */});
    },
    [],
  );

  if (!event) return null;

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onClick={onClose}
      onKeyDown={handleKeyDown}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="modal-title"
        className="modal-dialog"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleFocusTrap}
      >
        {/* Header */}
        <header className="modal-header">
          <div className="modal-title-block">
            <span className="eyebrow">EVENT INSPECTOR</span>
            <h2 id="modal-title" className="modal-title">
              {event.event_type.toUpperCase()} EVENT
              <span className="modal-id"> #{event.id}</span>
            </h2>
          </div>
          <button
            ref={closeButtonRef}
            type="button"
            className="modal-close"
            onClick={onClose}
            aria-label="Close inspector"
          >
            ✕
          </button>
        </header>

        {/* Meta row */}
        <div className="modal-meta">
          <MetaItem label="Contract" value={event.contract_id} mono />
          <MetaItem label="Ledger" value={event.ledger.toLocaleString()} mono />
          <MetaItem
            label="Timestamp"
            value={new Date(event.timestamp).toLocaleString()}
          />
          <MetaItem
            label="TX Hash"
            value={event.tx_hash.slice(0, 16) + '…'}
            mono
            title={event.tx_hash}
          />
        </div>

        {/* Body */}
        <div className="modal-body">
          {/* Topics */}
          <section className="modal-section">
            <div className="modal-section-header">
              <h3 className="modal-section-title">
                Topics
                <span className="modal-section-count">
                  {event.topics.length}
                </span>
              </h3>
              <button
                type="button"
                className="copy-btn"
                onClick={() => copyToClipboard(topicsJson, 'topics')}
                aria-label="Copy topics JSON"
              >
                Copy
              </button>
            </div>
            <div className="modal-topics">
              {decoded?.topics.map((topic, i) => (
                <div key={i} className="modal-topic-item">
                  <span className="modal-topic-index">{i + 1}</span>
                  <pre
                    className="json-block"
                    dangerouslySetInnerHTML={{
                      __html: highlight(prettyPrintScVal(topic)),
                    }}
                    aria-label={`Topic ${i + 1}`}
                  />
                </div>
              ))}
            </div>
          </section>

          {/* Value */}
          <section className="modal-section">
            <div className="modal-section-header">
              <h3 className="modal-section-title">Value (XDR decoded)</h3>
              <button
                type="button"
                className="copy-btn"
                onClick={() => copyToClipboard(valueJson, 'value')}
                aria-label="Copy value JSON"
              >
                Copy
              </button>
            </div>
            <pre
              className="json-block json-block--value"
              dangerouslySetInnerHTML={{ __html: highlight(valueJson) }}
              aria-label="Decoded event value"
            />
          </section>

          {/* Raw XDR */}
          <details className="modal-section modal-raw">
            <summary className="modal-section-title">Raw XDR (base64)</summary>
            <div className="modal-raw-grid">
              <div>
                <h4>Topics</h4>
                {event.topics.map((t, i) => (
                  <pre key={i} className="json-block json-block--raw">{t}</pre>
                ))}
              </div>
              <div>
                <h4>Value</h4>
                <pre className="json-block json-block--raw">{event.value_xdr}</pre>
              </div>
            </div>
          </details>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

function MetaItem({
  label,
  value,
  mono = false,
  title,
}: {
  label: string;
  value: string;
  mono?: boolean;
  title?: string;
}) {
  return (
    <div className="modal-meta-item">
      <span className="modal-meta-label">{label}</span>
      <span
        className={`modal-meta-value${mono ? ' mono' : ''}`}
        title={title}
      >
        {value}
      </span>
    </div>
  );
}
