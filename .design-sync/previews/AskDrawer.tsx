import { AskDrawer } from '@takomo/web'

const noop = () => {}
const L = {
  title: 'Ask a human',
  subtitle: 'Hand a decision to a person. It lands in the inbox and, if blocking, parks this ticket until it is answered.',
  fTicket: 'Ticket', fKind: 'Kind', fMode: 'Mode', fTitle: 'Question', fBody: 'Context',
  fOptions: 'Options', fOptionsHint: 'One per line — only for a choose question.',
  fExpertise: 'Expertise', fExpertiseHint: 'Comma-separated, e.g. domain:billing. Only holders of a matching scope can answer.',
  blocking: 'Blocking', advisory: 'Advisory',
  blockingHint: 'Parks the ticket and releases the agent’s claim. It resumes when every blocking question is answered.',
  advisoryHint: 'Records a routed decision. The ticket keeps moving.',
  langHint: 'This project asks questions in {lang}.',
  ask: 'Ask', cancel: 'Cancel', needTitle: 'Write the question first.',
}

/**
 * The consequential choice — blocking or advisory — is asked first and explains
 * its own consequence, because the two do very different things to the ticket.
 */
export function Open() {
  return (
    <AskDrawer
      open onOpenChange={noop} ticket="demo-2cx4" languageHint="German"
      onAsk={async () => {}} labels={L}
    />
  )
}
