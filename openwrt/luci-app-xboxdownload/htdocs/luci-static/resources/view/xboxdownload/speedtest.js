'use strict';
'require view';
'require uci';
'require poll';
'require xboxdownload as xb';

var stages = { idle: _('Idle'), loading: _('Loading candidates'), baseline: _('Testing the current node'),
	prefiltering: _('Measuring TCP latency'), downloading: _('Downloading test ranges'), completed: _('Completed'),
	cancelled: _('Cancelled'), failed: _('Failed') };

function select(label, choices) {
	return E('select', { 'aria-label': label }, Object.keys(choices).map(function(v) {
		return E('option', { value: v }, xb.text(choices[v]));
	}));
}

return view.extend({
	load: function() { return Promise.all([ xb.speedtest_status(), uci.load('xboxdownload') ]); },
	render: function(data) {
		var job = data[0], writable = !!L.hasViewPermission(), seq = 0, editSeq = 0;
		var group = select(_('Group'), xb.groups), family = select(_('Address family'), xb.families);
		var keyword = E('input', { type: 'text', maxlength: 256, placeholder: _('Location or carrier') });
		var sort = select(_('Sort results'), { speed: _('Fastest download'), rtt: _('Lowest latency') });
		var current = E('div', { 'aria-live': 'polite' }), results = E('div'), network = E('p', { role: 'status' });
		var custom = E('textarea', { rows: 5, 'aria-label': _('Custom IP list'), spellcheck: 'false' });
		var customStatus = E('p', { role: 'status' }), editor = E('details');
		var drafts = Object.create(null), loadedGroup = null;
		function editGroup() {
			var id = group.value, version = ++editSeq;
			loadedGroup = null; custom.disabled = true; custom.value = ''; save.disabled = true;
			if (!editor.open) return Promise.resolve();
			function show(value) {
				if (version !== editSeq) return;
				custom.value = value; loadedGroup = id; custom.disabled = !writable; save.disabled = !writable;
				customStatus.textContent = '';
			}
			if (Object.hasOwnProperty.call(drafts, id)) { show(drafts[id]); return Promise.resolve(); }
			customStatus.textContent = _('Loading…');
			return xb.list_ips(id, 'dual', '').then(function(list) {
				if (typeof list.custom_text !== 'string') throw new Error(_('Custom list unavailable'));
				show(list.custom_text);
			}).catch(function(e) { if (version === editSeq) customStatus.textContent = e.message; });
		}
		custom.addEventListener('input', function() { if (loadedGroup) drafts[loadedGroup] = custom.value; });
		var save = xb.button(_('Save custom IPs'), function() {
			var id = loadedGroup, body = custom.value;
			if (!id || id !== group.value) return;
			return xb.save_custom_ips(id, body).then(function() { xb.notify(_('Custom list saved.')); });
		}, '', !writable);
		editor.append(E('summary', {}, _('Custom IP list')), E('p', {}, _('Enter one IP per line, optionally followed by a tab and a note in parentheses.')),
			custom, customStatus, save);
		editor.addEventListener('toggle', editGroup);
		function defaults() {
			family.value = uci.get('xboxdownload', group.value, 'ip_family') || 'ipv4';
			keyword.value = uci.get('xboxdownload', group.value, 'filter_keyword') || '';
		}
		if (job.group && xb.groups[job.group]) group.value = job.group;
		defaults();
		group.addEventListener('change', function() { defaults(); editGroup(); });
		var start = xb.button(_('Start speed test'), function() {
			return xb.speedtest_start(group.value, family.value, keyword.value.trim(), false).then(refresh);
		}, 'cbi-button-positive', !writable);
		var cancel = xb.button(_('Cancel speed test'), function() { return xb.speedtest_cancel().then(refresh); }, '', !writable);
		var syncResult = E('div', { 'aria-live': 'polite' });
		var sync = xb.button(_('Sync online IP lists'), function() {
			var mirror = uci.get('xboxdownload', 'speedtest', 'ip_sync_mirror') || 'github';
			syncResult.textContent = _('Syncing IP lists…');
			return xb.sync_ips(mirror, uci.get('xboxdownload', 'speedtest', 'ip_sync_custom_base') || '').then(function(r) {
				syncResult.replaceChildren(xb.table([ _('File'), _('Addresses'), _('Result') ], Object.entries(r.files || {}).map(function(entry) {
					var f = entry[1]; return [ entry[0], f.count, f.ok ? _('Synced') : f.error || _('Failed') ];
				}), _('No file results returned.')));
			}).catch(function(e) { syncResult.textContent = e.message; });
		}, '', !writable);
		function apply(id, row) {
			// Preserve the other address family from the live group, including scheduler changes.
			var active;
			return xb.status().then(function(s) {
				var g = s.groups && s.groups[id];
				if (!g) throw new Error(_('Current group settings unavailable'));
				active = g.enabled && (g.ip_family === row.family || g.ip_family === 'dual');
				return xb.apply_ip(id, row.family === 'ipv4' ? row.ip : g.selected_ipv4 || '',
					row.family === 'ipv6' ? row.ip : g.selected_ipv6 || '');
			}).then(function() {
				var message = active ? _('Applied %s to %s.') : _('Saved %s for %s. Enable this group and address family in Settings to use it.');
				xb.notify(message.format(row.ip, xb.groups[id]));
			});
		}
		function draw() {
			start.hidden = !!job.running; cancel.hidden = !job.running;
			var total = job.stage === 'downloading' || job.stage === 'completed' ? job.total_to_download || job.total_candidates : job.total_candidates;
			var stage = stages[job.stage] || job.stage || _('Idle');
			var progress = E('progress', { max: total || 1, 'aria-label': stage });
			if (total > 0) progress.value = Math.min(job.tested_count || 0, total);
			current.replaceChildren(E('p', {}, xb.text([ xb.groups[job.group], stage,
				total > 0 ? (job.tested_count || 0) + '/' + total : '' ].filter(Boolean).join(' · '))));
			if (job.running) current.append(progress);
			if (job.error || job.note) current.append(E('p', {}, xb.text(job.error || job.note)));
			var id = job.group, rows = (job.results || []).slice().sort(function(a, b) {
				return sort.value === 'rtt' ? a.rtt_ms - b.rtt_ms : b.speed_mbps - a.speed_mbps || a.rtt_ms - b.rtt_ms;
			});
			results.replaceChildren(xb.table([ _('IP address'), _('Address family'), _('Location / carrier'), _('TCP latency'), _('Speed'), _('HTTP status'), _('Action') ],
				rows.map(function(r) { return [ r.ip, xb.families[r.family], r.location, xb.rtt(r.rtt_ms),
					r.http_status == null && !r.speed_mbps ? '—' : xb.speed(r.speed_mbps), r.http_status,
					xb.button(_('Apply'), function() { return apply(id, r); }, '', !writable || !r.ok || !xb.groups[id]) ]; }),
				_('No measurements yet.')));
		}
		function refresh() {
			var version = ++seq;
			return xb.speedtest_status().then(function(j) {
				if (version === seq) { job = j; draw(); network.textContent = ''; }
			}).catch(function(e) { if (version === seq) network.textContent = e.message; });
		}
		sort.addEventListener('change', draw);
		var tags = [ [ '', _('All') ], [ '电信', _('China Telecom') ], [ '联通', _('China Unicom') ], [ '移动', _('China Mobile') ],
			[ '香港', _('Hong Kong') ], [ '日本', _('Japan') ], [ '新加坡', _('Singapore') ] ];
		draw();
		var idleTicks = 0;
		poll.add(function() { if (job.running || ++idleTicks % 5 === 0) return refresh(); }, 1);
		return E('div', { 'class': 'xbox-page' }, [ xb.stylesheet(), E('h2', {}, _('Node speed test')), network,
			E('div', { 'class': 'xbox-fields' }, [ [ _('Group'), group ], [ _('Address family'), family ], [ _('Filter keyword'), keyword ] ]
				.map(function(pair) { return E('label', {}, [ xb.text(pair[0]), pair[1] ]); })),
			E('div', { 'class': 'xbox-actions' }, tags.map(function(t) { return xb.button(t[1], function() { keyword.value = t[0]; }); })),
			E('div', { 'class': 'xbox-actions' }, [ start, cancel, sync ]), syncResult, current,
			E('section', {}, [ E('h3', {}, _('Results')), E('label', { 'class': 'xbox-actions' }, [ xb.text(_('Sort results')), sort ]), results ]), editor ]);
	},
	handleSave: null, handleSaveApply: null, handleReset: null
});
