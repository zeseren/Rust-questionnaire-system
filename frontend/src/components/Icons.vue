<!-- 共享图标来自本地原型，所有符号只注入一次。 -->
<template>
  <svg width="0" height="0" aria-hidden="true" style="position: absolute; overflow: hidden">
    <defs>
      <symbol id="i-home" viewBox="0 0 24 24">
        <path d="m3 10 9-7 9 7M5 9v10a1 1 0 0 0 1 1h4v-6h4v6h4a1 1 0 0 0 1-1V9" />
      </symbol>
      <symbol id="i-grid" viewBox="0 0 24 24">
        <rect x="3.5" y="3.5" width="6.5" height="6.5" rx="1.5" />
        <rect x="14" y="3.5" width="6.5" height="6.5" rx="1.5" />
        <rect x="3.5" y="14" width="6.5" height="6.5" rx="1.5" />
        <rect x="14" y="14" width="6.5" height="6.5" rx="1.5" />
      </symbol>
      <symbol id="i-document" viewBox="0 0 24 24">
        <path
          d="M14 3H6a1.5 1.5 0 0 0-1.5 1.5v15A1.5 1.5 0 0 0 6 21h12a1.5 1.5 0 0 0 1.5-1.5V8.5L14 3Z"
        />
        <path d="M14 3v5.5h5.5M8 12h8M8 16h5" />
      </symbol>
      <symbol id="i-forms" viewBox="0 0 24 24">
        <rect x="6.5" y="5.5" width="14" height="15" rx="2" />
        <path
          d="M17 5.5V4a1.5 1.5 0 0 0-1.5-1.5h-11A2 2 0 0 0 2.5 4.5v11A1.5 1.5 0 0 0 4 17h2.5M10 10h7M10 14h7M10 17h4"
        />
      </symbol>
      <symbol id="i-chart" viewBox="0 0 24 24">
        <path d="M4 3v16a1 1 0 0 0 1 1h16M8 15v-4M13 15V6M18 15V9" />
      </symbol>
      <symbol id="i-archive" viewBox="0 0 24 24">
        <rect x="3" y="3.5" width="18" height="5" rx="1.5" />
        <path d="M5 8.5v11a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-11M9 12h6" />
      </symbol>
      <symbol id="i-chevron" viewBox="0 0 24 24"><path d="m6 9 6 6 6-6" /></symbol>
      <symbol id="i-plus" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14" /></symbol>
      <symbol id="i-search" viewBox="0 0 24 24">
        <circle cx="10.5" cy="10.5" r="6.5" />
        <path d="m15.5 15.5 5 5" />
      </symbol>
      <symbol id="i-list" viewBox="0 0 24 24">
        <path d="M9 5h11M9 12h11M9 19h11M4 5h.01M4 12h.01M4 19h.01" stroke-width="2" />
      </symbol>
      <symbol id="i-sun" viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="4" />
        <path
          d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5 19 19M5 19l1.5-1.5M17.5 6.5 19 5"
        />
      </symbol>
      <symbol id="i-moon" viewBox="0 0 24 24">
        <path d="M20.5 14.3A8.8 8.8 0 0 1 9.7 3.5a8.8 8.8 0 1 0 10.8 10.8Z" />
      </symbol>
      <symbol id="i-desktop" viewBox="0 0 24 24">
        <rect x="3" y="4" width="18" height="13" rx="2" />
        <path d="M8 21h8M12 17v4" />
      </symbol>
      <symbol id="i-arrow" viewBox="0 0 24 24"><path d="M4 12h15m-5-5 5 5-5 5" /></symbol>
      <symbol id="i-up-right" viewBox="0 0 24 24"><path d="M6 18 18 6M7 6h11v11" /></symbol>
      <symbol id="i-check" viewBox="0 0 24 24"><path d="m5 12 4.5 4.5L19 7" /></symbol>
      <symbol id="i-more" viewBox="0 0 24 24">
        <circle cx="5" cy="12" r=".75" fill="currentColor" />
        <circle cx="12" cy="12" r=".75" fill="currentColor" />
        <circle cx="19" cy="12" r=".75" fill="currentColor" />
      </symbol>
      <symbol id="i-person" viewBox="0 0 24 24">
        <circle cx="12" cy="7" r="3.5" />
        <path d="M5 20v-2a7 7 0 0 1 14 0v2H5Z" />
      </symbol>
      <symbol id="i-info" viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="9" />
        <path d="M12 11v6M12 7h.01" />
      </symbol>
      <symbol id="i-palette" viewBox="0 0 24 24">
        <path
          d="M12 3a9 9 0 0 0 0 18h1.5a2 2 0 0 0 1.4-3.4 1.4 1.4 0 0 1 1-2.4H18a3 3 0 0 0 3-3C21 7 17 3 12 3Z"
        />
        <circle cx="7" cy="11" r=".8" fill="currentColor" />
        <circle cx="10" cy="7" r=".8" fill="currentColor" />
        <circle cx="15" cy="7.5" r=".8" fill="currentColor" />
        <circle cx="17.5" cy="11.5" r=".8" fill="currentColor" />
      </symbol>
      <symbol id="i-calendar" viewBox="0 0 24 24">
        <rect x="3.5" y="5" width="17" height="16" rx="2" />
        <path d="M7 3v4M17 3v4M3.5 10h17M8 14h2M14 14h2M8 17h2" />
      </symbol>
      <symbol id="i-close" viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18" /></symbol>
      <symbol id="i-menu" viewBox="0 0 24 24"><path d="M4 6h16M4 12h16M4 18h16" /></symbol>
      <symbol id="i-cube" viewBox="0 0 24 24">
        <path d="m12 2.8 8.5 4.6v9.2L12 21.2l-8.5-4.6V7.4L12 2.8Z" />
        <path d="m3.5 7.4 8.5 4.7 8.5-4.7M12 12.1v9.1M7.7 5.1l8.6 4.6" />
      </symbol>
      <symbol id="i-school" viewBox="0 0 24 24">
        <path d="m2 8 10-5 10 5-10 5L2 8ZM6 10v6c3 3 9 3 12 0v-6M22 8v7" />
      </symbol>
      <symbol id="i-book" viewBox="0 0 24 24">
        <path
          d="M12 5c-3-2-6-2-9-1v15c3-1 6-1 9 1 3-2 6-2 9-1V4c-3-1-6-1-9 1Zm0 0v15M6 8c1-.2 2 0 3 .5M6 12c1-.2 2 0 3 .5M15 8.5c1-.5 2-.7 3-.5M15 12.5c1-.5 2-.7 3-.5"
        />
      </symbol>
      <symbol id="i-people" viewBox="0 0 24 24">
        <circle cx="9" cy="7" r="3" />
        <path d="M3 20v-2a6 6 0 0 1 12 0v2H3ZM16 4a3 3 0 0 1 0 6M18 13a5 5 0 0 1 3 5v2h-3" />
      </symbol>
      <symbol id="i-cup" viewBox="0 0 24 24">
        <path
          d="M4 8h13v7a5 5 0 0 1-5 5H9a5 5 0 0 1-5-5V8ZM17 9h1a3 3 0 0 1 0 6h-1M8 3v2M12 2v3M3 22h16"
        />
      </symbol>
      <symbol id="i-spark" viewBox="0 0 24 24">
        <path d="m12 2 2.5 7.5L22 12l-7.5 2.5L12 22l-2.5-7.5L2 12l7.5-2.5L12 2Z" />
      </symbol>
      <symbol id="i-collect" viewBox="0 0 24 24">
        <path d="M12 3v11m-4-4 4 4 4-4M5 12l-2 8h18l-2-8M3.7 17H8l1.5 3m10.8-3H16l-1.5 3" />
      </symbol>
      <symbol id="i-logout" viewBox="0 0 24 24">
        <path d="M10 4H5v16h5M10 12h11m-4-4 4 4-4 4" />
      </symbol>
      <symbol id="i-copy" viewBox="0 0 24 24">
        <rect x="8" y="8" width="12" height="13" rx="1" />
        <path d="M16 8V3H3v13h5" />
      </symbol>
      <symbol id="i-trash" viewBox="0 0 24 24">
        <path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7" />
      </symbol>
      <symbol id="i-up" viewBox="0 0 24 24"><path d="M12 20V4m-6 6 6-6 6 6" /></symbol>
      <symbol id="i-down" viewBox="0 0 24 24"><path d="M12 4v16m-6-6 6 6 6-6" /></symbol>
      <symbol id="i-edit" viewBox="0 0 24 24">
        <path d="m14 4 6 6M4 20l5-1L21 7l-4-4L5 15l-1 5Z" />
      </symbol>
    </defs>
  </svg>
</template>
