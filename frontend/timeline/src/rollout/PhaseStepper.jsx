import { Fragment } from 'react';
import { PHASES, stepStates } from './phases.js';

const STEP_ICON = { done: '✓', blocked: '!', active: '•', pending: '' };

/**
 * Horizontal step-by-step visualizer for the Stellar initialization pipeline:
 * Database Schema Migration → History Catchup → Quorum Peering → Fully Synced.
 * Completed steps show a check, the active step pulses, a blocked step is
 * flagged red, and future steps stay muted. Layout mirrors the Argo Rollouts
 * step list: labelled nodes joined by connectors.
 */
export default function PhaseStepper({ replica }) {
  const states = stepStates(replica);
  return (
    <ol className="stepper" aria-label={`Initialization phases for ${replica.name}`}>
      {PHASES.map((phase, index) => (
        <Fragment key={phase.id}>
          {index > 0 ? (
            <li aria-hidden="true" className={`stepper-connector ${states[index - 1] !== 'pending' && states[index] !== 'pending' ? 'done' : ''}`} />
          ) : null}
          <li className={`stepper-step step-${states[index]} ${phase.tone}`} title={phase.description}>
            <span className="step-dot">{STEP_ICON[states[index]] ?? index + 1}</span>
            <span className="step-label">{phase.label}</span>
          </li>
        </Fragment>
      ))}
    </ol>
  );
}
