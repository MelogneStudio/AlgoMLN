import React from 'react'
import { createRoot } from 'react-dom/client'

import { App } from './App'
import { ToastProvider } from './components/Toast/ToastContext'
import './styles/tokens.css'
import './styles/fonts.css'
import './styles/global.css'

createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ToastProvider>
      <App />
    </ToastProvider>
  </React.StrictMode>
)
