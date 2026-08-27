// The single entry point for all four surfaces.
//
// There used to be four of these, one per document. Now there is one bundle and
// a router, so React and the component library are downloaded once and moving
// between /board and /inbox keeps them warm.
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { ToastProvider } from '@/components/Toaster'
import { App } from './App'
import './styles/globals.css'
import './styles/markdown.css'
import './styles/editor.css'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ToastProvider>
      <App />
    </ToastProvider>
  </StrictMode>,
)
