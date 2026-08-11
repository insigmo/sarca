import Typography from '@suid/material/Typography'
import Box from '@suid/material/Box'
import Button from '@suid/material/Button'
import { A } from '@solidjs/router'
import { t } from '../common/i18n'

const NotFound = () => {
	return (
		<Box
			sx={{
				display: 'flex',
				alignItems: 'center',
				justifyContent: 'center',
				flexDirection: 'column',
				mt: 12,
				gap: 1.5,
				textAlign: 'center',
			}}
		>
			<Typography
				variant="h1"
				sx={{
					fontFamily: 'var(--sarca-display)',
					fontSize: { xs: '4.5rem', sm: '6rem' },
					color: 'primary.dark',
					lineHeight: 1,
				}}
			>
				404
			</Typography>
			<Typography variant="h5" color="text.secondary">
				{t('auth.notFound.message')}
			</Typography>
			<Button component={A} href="/storages" variant="contained" color="secondary" sx={{ mt: 2 }}>
				{t('auth.notFound.backToStorages')}
			</Button>
		</Box>
	)
}

export default NotFound
