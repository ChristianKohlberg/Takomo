// Workflow-graph helpers.
//
// Convenience verbs (start / done / block / cancel) must work across different
// workflows (e.g. `simple` uses in_progress/blocked, `factory-default` uses
// implementing/needs-decision). Rather than hard-coding state names, we read the
// project's workflow definition and resolve targets by *category*, choosing
// among the transitions actually legal from the ticket's current state.

import { TakomoClient } from "./client.js";

export interface WfState {
  id: string;
  category: string;
  claimable: boolean;
  terminal: boolean;
}
export interface WfTransition {
  from: string;
  to: string;
  requires?: string[];
}
export interface Workflow {
  name: string;
  initial: string;
  states: WfState[];
  transitions: WfTransition[];
}

const cache = new Map<string, Workflow>();

export async function getWorkflow(client: TakomoClient, project: string): Promise<Workflow> {
  const cached = cache.get(project);
  if (cached) return cached;
  const wf = (await client.request<Workflow>({
    path: `/projects/${encodeURIComponent(project)}/workflow`,
  })) as Workflow;
  cache.set(project, wf);
  return wf;
}

export function isClaimable(wf: Workflow, stateId: string): boolean {
  return wf.states.find((s) => s.id === stateId)?.claimable ?? false;
}

export function categoryOf(wf: Workflow, stateId: string): string | undefined {
  return wf.states.find((s) => s.id === stateId)?.category;
}

// The legal target states in a given category, reachable from `fromState`.
export function targetsInCategory(wf: Workflow, fromState: string, category: string): string[] {
  const byId = new Map(wf.states.map((s) => [s.id, s]));
  return wf.transitions
    .filter((t) => t.from === fromState && byId.get(t.to)?.category === category)
    .map((t) => t.to);
}
