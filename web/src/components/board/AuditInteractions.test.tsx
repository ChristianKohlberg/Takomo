import { describe, it, expect, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { AskDrawer } from './AskDrawer'
import { DetailPanel } from './DetailPanel'
import { STR } from '@/pages/board/strings'

const t = STR.en
const askLabels = { title: t.askHuman, subtitle: t.questionIntro, fTicket: t.refLabel, fKind: t.questionType, fMode: t.questionMode, fTitle: t.questionTitle, fBody: t.questionNote, fOptions: t.answerChoices, fOptionsHint: t.answerChoicesHint, fExpertise: t.expertiseLabel, fExpertiseHint: t.expertiseHint, fAssignee: t.askAssignee, fAssigneeHint: t.askAssigneeHint, fAssigneeAnyone: t.askAssigneeAnyone, blocking: t.blocking, advisory: t.advisory, blockingHint: t.answeringResumes, advisoryHint: t.decisionRouted, langHint: t.askLangHint, ask: t.send, cancel: t.cancel, needTitle: t.typeFirst }

describe('audited ticket interactions', () => {
  it('requires a question and two choices before sending a choice question', () => {
    const onAsk = vi.fn()
    render(<AskDrawer open ticket="demo-1" onOpenChange={vi.fn()} onAsk={onAsk} labels={askLabels} kindLabels={{ confirm: t.kindConfirm, choose: t.kindChoose, clarify: t.kindClarify, approve: t.kindApprove }} />)
    expect(screen.getByRole('button', { name: t.send })).toHaveProperty('disabled', true)
    fireEvent.change(screen.getByLabelText(t.questionTitle), { target: { value: 'When should we start?' } })
    fireEvent.click(screen.getByRole('combobox', { name: t.questionType }))
    fireEvent.click(screen.getByRole('option', { name: t.kindChoose }))
    expect(screen.getByText(t.answerChoicesHint)).toBeTruthy()
    expect(screen.getByRole('button', { name: t.send })).toHaveProperty('disabled', true)
    fireEvent.change(screen.getByLabelText(t.answerChoices), { target: { value: 'Now, Friday' } })
    fireEvent.click(screen.getByRole('button', { name: t.send }))
    expect(onAsk).toHaveBeenCalledWith(expect.objectContaining({ kind: 'choose', title: 'When should we start?', options: ['Now', 'Friday'] }))
  })
  it('lets readers open child work from an epic detail', () => {
    const onOpenTicket = vi.fn()
    render(<DetailPanel ticket={{ id: 'demo-e', title: 'Launch', type: 'epic', state: 'todo', project: 'demo' }} labels={t} canAsk={false} onAsk={vi.fn()} onClose={vi.fn()} onOpenTicket={onOpenTicket} terminalStates={['done']} relatedTickets={[{ id: 'demo-child', title: 'Ship editor', state: 'done', project: 'demo' }]} navigationLabels={{ copyLink: t.copyLink, copiedLink: t.copiedLink, linkFailed: t.linkFailed, childTickets: t.childTickets, overview: t.overview, activity: t.activity }} />)
    expect(screen.getByRole('heading', { name: 'Child tickets (1/1)' })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Ship editor' }))
    expect(onOpenTicket).toHaveBeenCalledWith('demo-child')
    expect(screen.getByRole('button', { name: 'Copy link' })).toBeTruthy()
  })
})
